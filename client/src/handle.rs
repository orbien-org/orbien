use crate::local_control;
use crate::reload::ReloadOutcome;
use crate::service::{ReloadRequest, Service};
use anyhow::{anyhow, bail, Result};
use orbien_core::config::ClientConfig;
use orbien_core::transport::Protocol;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

const UDP_SESSION_DRAIN: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    Stopped,
    Starting,
    Running,
    Reconnecting,
    Stopping,
}

impl ClientStatus {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Reconnecting | Self::Stopping
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub local_control: bool,
}

#[derive(Debug, Default)]
struct TunnelRemoteState {
    gen: u64,
    by_name: std::collections::HashMap<String, String>,
}

struct Inner {
    status: Mutex<ClientStatus>,
    last_error: Mutex<Option<String>>,
    pending_logs: Mutex<Vec<String>>,
    tunnel_remotes: Mutex<TunnelRemoteState>,
    cancel: Mutex<Option<CancellationToken>>,
    join: Mutex<Option<JoinHandle<()>>>,
    reload_tx: Mutex<Option<mpsc::Sender<ReloadRequest>>>,
    reload_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct ClientHandle {
    inner: Arc<Inner>,
}

impl Default for ClientHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                status: Mutex::new(ClientStatus::Stopped),
                last_error: Mutex::new(None),
                pending_logs: Mutex::new(Vec::new()),
                tunnel_remotes: Mutex::new(TunnelRemoteState::default()),
                cancel: Mutex::new(None),
                join: Mutex::new(None),
                reload_tx: Mutex::new(None),
                reload_lock: tokio::sync::Mutex::new(()),
            }),
        }
    }

    pub fn status(&self) -> ClientStatus {
        *self.inner.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn last_error(&self) -> Option<String> {
        self.inner
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn take_last_error(&self) -> Option<String> {
        self.inner
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn push_log(&self, line: impl Into<String>) {
        self.enqueue_log(line.into());
    }

    pub fn drain_logs(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .inner
                .pending_logs
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        )
    }

    pub fn tunnel_remotes_if_changed(
        &self,
        since_gen: u64,
    ) -> Option<(u64, std::collections::HashMap<String, String>)> {
        let g = self
            .inner
            .tunnel_remotes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if g.gen == since_gen {
            return None;
        }
        Some((g.gen, g.by_name.clone()))
    }

    pub fn clear_tunnel_remotes(&self) {
        let mut g = self
            .inner
            .tunnel_remotes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        g.by_name.clear();
        g.gen = g.gen.wrapping_add(1);
    }

    fn set_tunnel_remote(&self, name: String, remote_addr: String) {
        if name.is_empty() {
            return;
        }
        let mut g = self
            .inner
            .tunnel_remotes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if g.by_name.get(&name) == Some(&remote_addr) {
            return;
        }
        g.by_name.insert(name, remote_addr);
        g.gen = g.gen.wrapping_add(1);
    }

    fn remove_tunnel_remote(&self, name: String) {
        if name.is_empty() {
            return;
        }
        let mut g = self
            .inner
            .tunnel_remotes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if g.by_name.remove(&name).is_some() {
            g.gen = g.gen.wrapping_add(1);
        }
    }

    fn enqueue_log(&self, line: String) {
        if let Ok(mut g) = self.inner.pending_logs.lock() {
            const MAX_PENDING: usize = 500;
            if g.len() >= MAX_PENDING {
                let drop_n = g.len() - MAX_PENDING + 1;
                g.drain(0..drop_n);
            }
            g.push(line);
        }
    }

    fn set_status(&self, s: ClientStatus) {
        if let Ok(mut g) = self.inner.status.lock() {
            *g = s;
        }
    }

    fn set_error(&self, e: Option<String>) {
        if let Ok(mut g) = self.inner.last_error.lock() {
            *g = e;
        }
    }

    fn clear_reload_tx(&self) {
        *self
            .inner
            .reload_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn set_reload_tx(&self, tx: mpsc::Sender<ReloadRequest>) {
        *self
            .inner
            .reload_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(tx);
    }

    pub async fn run_foreground(
        self,
        cfg: ClientConfig,
        config_path: PathBuf,
        opts: StartOptions,
    ) -> Result<()> {
        self.run_inner(cfg, config_path, opts, true).await
    }

    pub fn start_with(
        &self,
        mut cfg: ClientConfig,
        config_path: PathBuf,
        opts: StartOptions,
    ) -> Result<()> {
        if self.status().is_active() {
            bail!("client already running");
        }
        cfg.prepare_runtime(&config_path);
        cfg.validate()?;

        let cancel = CancellationToken::new();
        *self.inner.cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
        self.set_status(ClientStatus::Starting);
        self.set_error(None);
        self.clear_tunnel_remotes();

        let handle = self.clone();
        let join = tokio::spawn(async move {
            let result = handle.run_inner(cfg, config_path, opts, false).await;
            if let Err(e) = &result {
                tracing::error!(error = %e, "client service ended with error");
                handle.set_error(Some(e.to_string()));
            }
            handle.clear_tunnel_remotes();
            handle.clear_reload_tx();
            *handle
                .inner
                .cancel
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
            handle.set_status(ClientStatus::Stopped);
            let _ = result;
        });
        *self.inner.join.lock().unwrap_or_else(|e| e.into_inner()) = Some(join);
        Ok(())
    }

    pub fn start(&self, cfg: ClientConfig, config_path: PathBuf) -> Result<()> {
        self.start_with(cfg, config_path, StartOptions::default())
    }

    async fn run_inner(
        &self,
        mut cfg: ClientConfig,
        config_path: PathBuf,
        opts: StartOptions,
        is_foreground: bool,
    ) -> Result<()> {
        cfg.prepare_runtime(&config_path);
        cfg.validate()?;

        let cancel = if is_foreground {
            let token = CancellationToken::new();
            *self.inner.cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.clone());
            token
        } else {
            self.inner
                .cancel
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .ok_or_else(|| anyhow!("missing cancel token"))?
        };

        let drain_udp = is_foreground && needs_udp_session_drain(&cfg);
        if drain_udp {
            let token = cancel.clone();
            tokio::spawn(async move {
                if wait_termination_signal().await {
                    tracing::info!("termination signal received; draining udp session");
                    token.cancel();
                }
            });
        }

        if is_foreground {
            self.set_status(ClientStatus::Starting);
            self.set_error(None);
            self.clear_tunnel_remotes();
        }

        let (reload_tx, mut reload_rx) = mpsc::channel(4);
        self.set_reload_tx(reload_tx);
        let shared_cfg = Arc::new(RwLock::new(cfg.clone()));

        if opts.local_control {
            let socket_path = local_control::default_socket_path();
            let hc = self.clone();
            let cc = cancel.clone();
            tokio::spawn(async move {
                if let Err(e) = local_control::serve(socket_path, hc, cc).await {
                    tracing::warn!(error = %e, "local control socket stopped");
                }
            });
        }

        let on_status = {
            let h = self.clone();
            move |st| h.set_status(st)
        };
        let on_log = {
            let h = self.clone();
            move |line: String| h.enqueue_log(line)
        };
        let on_tunnel_remote: Arc<dyn Fn(String, String) + Send + Sync> = {
            let h = self.clone();
            Arc::new(move |name, remote| h.set_tunnel_remote(name, remote))
        };
        let on_tunnel_removed: Arc<dyn Fn(String) + Send + Sync> = {
            let h = self.clone();
            Arc::new(move |name| h.remove_tunnel_remote(name))
        };
        let on_remotes_clear: Arc<dyn Fn() + Send + Sync> = {
            let h = self.clone();
            Arc::new(move || h.clear_tunnel_remotes())
        };

        let result = Service::new(cfg)
            .run(
                cancel.clone(),
                &mut reload_rx,
                shared_cfg,
                on_status,
                on_log,
                on_tunnel_remote,
                on_tunnel_removed,
                on_remotes_clear,
            )
            .await;

        if drain_udp {
            sleep(UDP_SESSION_DRAIN).await;
        }

        if is_foreground {
            self.clear_tunnel_remotes();
            self.clear_reload_tx();
            *self.inner.cancel.lock().unwrap_or_else(|e| e.into_inner()) = None;
            if let Err(ref e) = result {
                self.set_error(Some(e.to_string()));
            }
            self.set_status(ClientStatus::Stopped);
        }
        result
    }

    pub async fn reload(&self, cfg: ClientConfig, config_path: PathBuf) -> Result<ReloadOutcome> {
        if !self.status().is_active() {
            bail!("client is not running");
        }
        let _guard = self.inner.reload_lock.lock().await;
        self.send_reload(cfg, config_path).await
    }

    pub async fn reload_from_path(&self, path: &Path) -> Result<ReloadOutcome> {
        let cfg = ClientConfig::load(path)?;
        self.reload(cfg, path.to_path_buf()).await
    }

    async fn send_reload(&self, cfg: ClientConfig, config_path: PathBuf) -> Result<ReloadOutcome> {
        let tx = self
            .inner
            .reload_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| anyhow!("reload channel not ready"))?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(ReloadRequest {
            cfg,
            config_path,
            reply: reply_tx,
        })
        .await
        .map_err(|_| anyhow!("reload channel closed"))?;
        match tokio::time::timeout(Duration::from_secs(120), reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(anyhow!("reload response channel closed")),
            Err(_) => Err(anyhow!("reload timed out after 120s")),
        }
    }

    pub async fn stop(&self) {
        if matches!(self.status(), ClientStatus::Stopped) {
            return;
        }
        self.set_error(None);
        self.set_status(ClientStatus::Stopping);
        if let Some(token) = self
            .inner
            .cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            token.cancel();
        }
        let join = self
            .inner
            .join
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(join) = join {
            let abort = join.abort_handle();
            match tokio::time::timeout(std::time::Duration::from_secs(5), join).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "client task join error"),
                Err(_) => {
                    tracing::warn!("client stop timed out after 5s, aborting task");
                    abort.abort();
                    self.set_status(ClientStatus::Stopped);
                }
            }
        } else {
            self.set_status(ClientStatus::Stopped);
        }
        self.clear_tunnel_remotes();
        self.clear_reload_tx();
    }
}

fn needs_udp_session_drain(cfg: &ClientConfig) -> bool {
    matches!(cfg.protocol().ok(), Some(Protocol::Quic | Protocol::Kcp))
}

async fn wait_termination_signal() -> bool {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to install SIGTERM handler");
                return false;
            }
        };
        tokio::select! {
            res = tokio::signal::ctrl_c() => {
                if let Err(e) = res {
                    tracing::warn!(error = %e, "failed to wait for Ctrl-C");
                    return false;
                }
                true
            }
            _ = sigterm.recv() => true,
        }
    }
    #[cfg(not(unix))]
    {
        match tokio::signal::ctrl_c().await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "failed to wait for Ctrl-C");
                false
            }
        }
    }
}
