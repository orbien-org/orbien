use crate::control::{ActiveSession, Control, LoginRejected, SessionEnd};
use crate::handle::ClientStatus;
use crate::reload::{
    empty_outcome, outcome_from_plan, outcome_level, plan_reload, ReloadOutcome, ReloadPlan,
};
use anyhow::{anyhow, Result};
use orbien_core::config::ClientConfig;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

const RECONNECT_BASE_SECS: u64 = 1;
const RECONNECT_MAX_SECS: u64 = 60;

pub struct ReloadRequest {
    pub cfg: ClientConfig,
    pub config_path: PathBuf,
    pub reply: oneshot::Sender<Result<ReloadOutcome>>,
}

struct PendingReloadReply {
    reply: oneshot::Sender<Result<ReloadOutcome>>,
    outcome: ReloadOutcome,
}

pub struct Service {
    cfg: ClientConfig,
}

impl Service {
    pub fn new(cfg: ClientConfig) -> Self {
        if cfg.auth.token.is_empty() {
            tracing::warn!("auth.token is empty; authentication is disabled");
        }
        Self { cfg }
    }

    pub async fn run(
        self,
        cancel: CancellationToken,
        reload_rx: &mut mpsc::Receiver<ReloadRequest>,
        cfg: Arc<RwLock<ClientConfig>>,
        mut on_status: impl FnMut(ClientStatus),
        mut on_log: impl FnMut(String),
        on_tunnel_remote: Arc<dyn Fn(String, String) + Send + Sync>,
        on_tunnel_removed: Arc<dyn Fn(String) + Send + Sync>,
        on_remotes_clear: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<()> {
        {
            let mut guard = cfg.write().await;
            *guard = self.cfg;
        }

        let mut session_id = String::new();

        let mut first_attempt = true;
        let mut backoff_secs = RECONNECT_BASE_SECS;
        let mut pending_reload_reply: Option<PendingReloadReply> = None;

        loop {
            if cancel.is_cancelled() {
                fail_pending_reload(&mut pending_reload_reply, "client stopped during reconnect");
                tracing::info!("client service cancelled");
                return Ok(());
            }

            drain_reload_requests(reload_rx, &cfg, None, &mut on_log).await;

            on_remotes_clear();

            if first_attempt {
                on_status(ClientStatus::Starting);
                on_log("INFO  connecting to server".into());
            } else if pending_reload_reply.is_none() {
                on_status(ClientStatus::Reconnecting);
            }

            let connect_cancel = cancel.child_token();
            let session = tokio::select! {
                _ = cancel.cancelled() => {
                    fail_pending_reload(&mut pending_reload_reply, "client stopped during reconnect");
                    return Ok(());
                }
                req = reload_rx.recv() => {
                    if let Some(req) = req {
                        serve_reload_request(req, &cfg, None, &mut on_log).await;
                    }
                    continue;
                }
                res = Control::open_session(
                    Arc::clone(&cfg),
                    session_id.clone(),
                    connect_cancel,
                    || {
                        on_status(ClientStatus::Running);
                        on_log("INFO  connected to server".into());
                    },
                    Arc::clone(&on_tunnel_remote),
                    Arc::clone(&on_tunnel_removed),
                ) => res,
            };

            let session = match session {
                Ok(s) => s,
                Err(e) => {
                    if cancel.is_cancelled() {
                        fail_pending_reload(
                            &mut pending_reload_reply,
                            "client stopped during reconnect",
                        );
                        return Ok(());
                    }
                    if let Some(rej) = e.downcast_ref::<LoginRejected>() {
                        fail_pending_reload(&mut pending_reload_reply, &rej.to_string());
                        tracing::error!(reason = %rej.reason, "login rejected, stopping");
                        on_log(format!("ERROR {rej}"));
                        return Err(e);
                    }
                    on_log(format!("ERROR failed to connect: {e}"));
                    tracing::warn!(error = %e, "connect failed, retrying");
                    on_status(ClientStatus::Reconnecting);
                    first_attempt = false;
                    let delay = backoff_secs;
                    on_log(format!("INFO  retrying in {delay}s"));
                    if wait_or_reload(reload_rx, &cfg, &cancel, delay, &mut on_log).await {
                        fail_pending_reload(
                            &mut pending_reload_reply,
                            "client stopped during reconnect",
                        );
                        return Ok(());
                    }
                    backoff_secs = backoff_secs
                        .saturating_mul(2)
                        .clamp(RECONNECT_BASE_SECS, RECONNECT_MAX_SECS);
                    continue;
                }
            };

            first_attempt = false;
            backoff_secs = RECONNECT_BASE_SECS;

            if let Some(pending) = pending_reload_reply.take() {
                let _ = pending.reply.send(Ok(pending.outcome));
            }

            let end = run_session_loop(session, reload_rx, &cfg, &cancel, &mut on_log).await;

            on_remotes_clear();

            match end {
                SessionLoopEnd::Cancelled => {
                    fail_pending_reload(
                        &mut pending_reload_reply,
                        "client stopped during reconnect",
                    );
                    return Ok(());
                }
                SessionLoopEnd::ReloadReconnect(pending) => {
                    pending_reload_reply = Some(pending);
                    on_log("INFO  reconnecting to apply client settings".into());
                    on_status(ClientStatus::Reconnecting);
                    continue;
                }
                SessionLoopEnd::Session(end) => match end {
                    Ok(SessionEnd::Kicked {
                        session_id: rid,
                        reason,
                    }) => {
                        fail_pending_reload(&mut pending_reload_reply, "kicked by server");
                        tracing::warn!(
                            session_id = %rid,
                            %reason,
                            "kicked by server, stopping"
                        );
                        on_log(format!("WARN  kicked by server: {reason}"));
                        return Err(anyhow!("kicked by server: {reason}"));
                    }
                    Ok(SessionEnd::Disconnected { session_id: rid }) => {
                        if cancel.is_cancelled() {
                            fail_pending_reload(
                                &mut pending_reload_reply,
                                "client stopped during reconnect",
                            );
                            tracing::info!(session_id = %rid, "session ended after cancel");
                            return Ok(());
                        }
                        session_id = rid;
                        on_log("WARN  disconnected from server".into());
                        on_status(ClientStatus::Reconnecting);
                    }
                    Err(e) => {
                        if cancel.is_cancelled() {
                            fail_pending_reload(
                                &mut pending_reload_reply,
                                "client stopped during reconnect",
                            );
                            tracing::info!("session error after cancel: {e}");
                            return Ok(());
                        }
                        on_log(format!("ERROR session error: {e}"));
                        on_status(ClientStatus::Reconnecting);
                    }
                },
            }

            drain_reload_requests(reload_rx, &cfg, None, &mut on_log).await;

            let delay = backoff_secs;
            on_log(format!("INFO  retrying in {delay}s"));
            tracing::info!(delay_secs = delay, "reconnect backoff");

            if wait_or_reload(reload_rx, &cfg, &cancel, delay, &mut on_log).await {
                fail_pending_reload(&mut pending_reload_reply, "client stopped during reconnect");
                tracing::info!("client service cancelled during backoff");
                return Ok(());
            }

            backoff_secs = backoff_secs
                .saturating_mul(2)
                .clamp(RECONNECT_BASE_SECS, RECONNECT_MAX_SECS);
        }
    }
}

async fn wait_or_reload(
    reload_rx: &mut mpsc::Receiver<ReloadRequest>,
    cfg: &Arc<RwLock<ClientConfig>>,
    cancel: &CancellationToken,
    delay_secs: u64,
    on_log: &mut impl FnMut(String),
) -> bool {
    tokio::select! {
        _ = cancel.cancelled() => true,
        _ = sleep(Duration::from_secs(delay_secs)) => false,
        req = reload_rx.recv() => {
            if let Some(req) = req {
                serve_reload_request(req, cfg, None, on_log).await;
            }
            false
        }
    }
}

async fn drain_reload_requests(
    reload_rx: &mut mpsc::Receiver<ReloadRequest>,
    cfg: &Arc<RwLock<ClientConfig>>,
    control: Option<&Arc<Control>>,
    on_log: &mut impl FnMut(String),
) {
    while let Ok(req) = reload_rx.try_recv() {
        serve_reload_request(req, cfg, control, on_log).await;
    }
}

async fn serve_reload_request(
    req: ReloadRequest,
    cfg: &Arc<RwLock<ClientConfig>>,
    control: Option<&Arc<Control>>,
    on_log: &mut impl FnMut(String),
) {
    let result = match apply_reload(req.cfg, &req.config_path, cfg, control, on_log).await {
        ApplyReloadResult::Reply(result) => result,
        ApplyReloadResult::Reconnect { outcome } => Ok(outcome),
    };
    let _ = req.reply.send(result);
}

fn fail_pending_reload(pending: &mut Option<PendingReloadReply>, reason: &str) {
    if let Some(p) = pending.take() {
        let _ = p.reply.send(Err(anyhow::anyhow!("{}", reason)));
    }
}

enum SessionLoopEnd {
    Cancelled,
    Session(Result<SessionEnd>),
    ReloadReconnect(PendingReloadReply),
}

enum ApplyReloadResult {
    Reply(Result<ReloadOutcome>),
    Reconnect { outcome: ReloadOutcome },
}

async fn run_session_loop(
    session: ActiveSession,
    reload_rx: &mut mpsc::Receiver<ReloadRequest>,
    cfg: &Arc<RwLock<ClientConfig>>,
    cancel: &CancellationToken,
    on_log: &mut impl FnMut(String),
) -> SessionLoopEnd {
    let control = session.control.clone();
    let mut done = session.done;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                control.close_all_tunnels().await;
                control.request_disconnect();
                let _ = done.await;
                return SessionLoopEnd::Cancelled;
            }
            req = reload_rx.recv() => {
                let Some(req) = req else {
                    control.request_disconnect();
                    let end = done.await;
                    return SessionLoopEnd::Session(end.unwrap_or_else(|_| {
                        Ok(SessionEnd::Disconnected {
                            session_id: control.session_id().to_string(),
                        })
                    }));
                };
                match apply_reload(req.cfg, &req.config_path, cfg, Some(&control), on_log).await {
                    ApplyReloadResult::Reply(result) => {
                        let _ = req.reply.send(result);
                    }
                    ApplyReloadResult::Reconnect { outcome } => {
                        let _ = done.await;
                        return SessionLoopEnd::ReloadReconnect(PendingReloadReply {
                            reply: req.reply,
                            outcome,
                        });
                    }
                }
            }
            end = &mut done => {
                return match end {
                    Ok(r) => SessionLoopEnd::Session(r),
                    Err(_) => SessionLoopEnd::Session(Ok(SessionEnd::Disconnected {
                        session_id: control.session_id().to_string(),
                    })),
                };
            }
        }
    }
}

async fn apply_reload(
    mut new_cfg: ClientConfig,
    config_path: &Path,
    cfg: &Arc<RwLock<ClientConfig>>,
    control: Option<&Arc<Control>>,
    on_log: &mut impl FnMut(String),
) -> ApplyReloadResult {
    new_cfg.prepare_runtime(config_path);
    let validated = match new_cfg.validate() {
        Ok(()) => new_cfg,
        Err(e) => return ApplyReloadResult::Reply(Err(e)),
    };
    new_cfg = validated;

    let plan = {
        let old = cfg.read().await;
        plan_reload(&old, &new_cfg)
    };

    match &plan {
        ReloadPlan::Noop => ApplyReloadResult::Reply(Ok(empty_outcome(&plan))),
        ReloadPlan::Apply {
            changes,
            connection_settings_changed,
        } => {
            if *connection_settings_changed {
                let outcome = outcome_from_plan(&plan, changes);
                if !changes.is_empty() {
                    on_log(format!(
                        "INFO  reload pending: +{} -{} ~{}",
                        outcome.added.len(),
                        outcome.removed.len(),
                        outcome.updated.len()
                    ));
                }
                {
                    let mut guard = cfg.write().await;
                    *guard = new_cfg;
                }
                if let Some(control) = control {
                    on_log("INFO  client settings changed; reconnecting".into());
                    control.close_all_tunnels().await;
                    control.request_disconnect();
                    return ApplyReloadResult::Reconnect { outcome };
                }
                on_log("INFO  client settings updated".into());
                return ApplyReloadResult::Reply(Ok(outcome));
            }

            if let Some(control) = control {
                let mut o = control.apply_tunnel_changes(changes).await;
                o.level = outcome_level(&plan);
                {
                    let mut guard = cfg.write().await;
                    *guard = new_cfg;
                }
                on_log(format!(
                    "INFO  reload applied: +{} -{} ~{}",
                    o.added.len(),
                    o.removed.len(),
                    o.updated.len()
                ));
                ApplyReloadResult::Reply(Ok(o))
            } else {
                {
                    let mut guard = cfg.write().await;
                    *guard = new_cfg;
                }
                let mut o = outcome_from_plan(&plan, changes);
                o.level = outcome_level(&plan);
                on_log(format!(
                    "INFO  reload queued: +{} -{} ~{}",
                    o.added.len(),
                    o.removed.len(),
                    o.updated.len()
                ));
                ApplyReloadResult::Reply(Ok(o))
            }
        }
    }
}
