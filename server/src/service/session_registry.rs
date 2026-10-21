use super::agent_registry::{
    sanitize_wire_id, AgentConflict, AgentOnlineSpec, AgentRegisterError, MAX_AGENT_ID_LEN,
    MAX_SESSION_ID_LEN, MAX_USER_LEN,
};
use super::session_table::{self, remove_if_current, swap_in_locked};
use super::Service;
use crate::control::Control;
use anyhow::{anyhow, Result};
use orbien_core::auth;
use orbien_core::msg::{self, Login, LoginResp, Message, NewDataConn};
use orbien_core::transport::DynStream;
use orbien_core::VERSION;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

const CONFLICT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const TAKEOVER_REMOVE_TIMEOUT: Duration = Duration::from_secs(3);
const TAKEOVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const TEARDOWN_REMOVE_TIMEOUT: Duration = Duration::from_secs(3);
const TEARDOWN_TUNNEL_COUNT_TIMEOUT: Duration = Duration::from_secs(2);

impl Service {
    pub(crate) async fn register_control(
        self: Arc<Self>,
        stream: DynStream,
        login: Login,
        peer: SocketAddr,
    ) -> Result<()> {
        if let Err(reason) = auth::verify_login(
            &self.cfg.auth.token,
            &login.auth_digest,
            login.timestamp,
            auth::unix_now_secs(),
            &self.auth_replay,
        ) {
            tracing::warn!(%reason, %peer, "login rejected");
            let mut stream = stream;
            let _ = msg::write_msg(
                &mut stream,
                &Message::LoginResp(LoginResp {
                    version: VERSION.into(),
                    session_id: String::new(),
                    error: "authorization failed".into(),
                }),
            )
            .await;
            return Err(anyhow!("authorization failed"));
        }

        let user = match sanitize_wire_id(&login.user, MAX_USER_LEN) {
            Ok(u) => u,
            Err(msg) => {
                return reject_login(stream, msg).await;
            }
        };
        let agent_id = match sanitize_wire_id(&login.agent_id, MAX_AGENT_ID_LEN) {
            Ok(id) => id,
            Err(msg) => {
                return reject_login(stream, msg).await;
            }
        };
        let session_id = if login.session_id.trim().is_empty() {
            short_session_id()
        } else {
            match sanitize_wire_id(&login.session_id, MAX_SESSION_ID_LEN) {
                Ok(id) if !id.is_empty() => id,
                Ok(_) => short_session_id(),
                Err(msg) => {
                    return reject_login(stream, msg).await;
                }
            }
        };

        let max_pool = self.cfg.transport.max_conn_pool.max(0) as usize;
        let pool_count = (login.pool_count.max(0) as usize).min(max_pool);
        let client_ip = peer.ip().to_string();
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);

        let control = Control::new(
            session_id.clone(),
            generation,
            stream,
            self.cfg.clone(),
            pool_count,
            self.http_gw.clone(),
            self.https_gw.clone(),
            user.clone(),
            agent_id.clone(),
            login.hostname.clone(),
            login.os.clone(),
            login.arch.clone(),
            login.version.clone(),
            client_ip,
            Arc::clone(&self.metrics),
            Arc::clone(&self.tunnel_registry),
            Arc::clone(&self.tcp_ports),
            Arc::clone(&self.udp_ports),
        );
        let control = Arc::new(control);

        let (session_guard, previous) =
            swap_in_locked(&self.controls, &session_id, Arc::clone(&control)).await;

        if let Some(old) = previous {
            tracing::info!(
                %session_id,
                old_generation = old.generation,
                new_generation = generation,
                "replacing prior control session"
            );
            old.shutdown().await;
        }

        let spec = AgentOnlineSpec {
            user: user.clone(),
            agent_id: agent_id.clone(),
            session_id: session_id.clone(),
            generation,
            hostname: control.hostname.clone(),
            os: control.os.clone(),
            arch: control.arch.clone(),
            client_ip: control.client_ip.clone(),
            version: control.version.clone(),
        };

        match self.agents.try_online(spec.clone()) {
            Ok(_) => {}
            Err(AgentRegisterError::Conflict(conflict)) => {
                if !self.take_over_conflict(&conflict).await
                    || self.agents.try_online(spec).is_err()
                {
                    drop(session_guard);
                    return reject_control(
                        &self.controls,
                        &session_id,
                        &control,
                        format!("agent_id [{agent_id}] for user [{user}] is already online"),
                    )
                    .await;
                }
            }
        }

        if let Err(e) = control.send_login_ok(VERSION).await {
            self.agents.release(&session_id, generation, 0);
            drop(session_guard);
            let _ = timeout(
                TEARDOWN_REMOVE_TIMEOUT,
                remove_if_current(&self.controls, &session_id, &control),
            )
            .await;
            control.shutdown().await;
            return Err(e);
        }

        drop(session_guard);

        tracing::info!(
            %session_id,
            %agent_id,
            generation,
            %peer,
            pool = login.pool_count,
            "client logged in"
        );

        self.metrics.new_client(&session_id);

        let controls = Arc::clone(&self.controls);
        let agents = Arc::clone(&self.agents);
        let metrics = Arc::clone(&self.metrics);
        let rid = session_id.clone();
        let result = Arc::clone(&control).run().await;

        let tunnel_count = timeout(TEARDOWN_TUNNEL_COUNT_TIMEOUT, control.tunnel_count())
            .await
            .unwrap_or(0);
        control.shutdown().await;
        let removed = timeout(
            TEARDOWN_REMOVE_TIMEOUT,
            remove_if_current(&controls, &rid, &control),
        )
        .await
        .unwrap_or(false);
        if removed {
            agents.release(&rid, generation, tunnel_count);
        } else {
            let still_current = {
                let map = controls.lock().await;
                map.get(&rid)
                    .map(|e| Arc::ptr_eq(&e.control, &control))
                    .unwrap_or(false)
            };
            if !still_current {
                agents.force_release(&rid, Some(generation), tunnel_count);
            }
        }
        self.release_stale_tunnel_slots(&rid, generation);
        metrics.close_client();

        result
    }

    /// Replace an old session that is no longer serving traffic.
    ///
    /// A record can stay `online` in `agents` after its control teardown
    /// failed, which used to make every restart fail with "already online".
    /// We only take over when the old control is missing, closed, heartbeat
    /// stale, or has no tunnels; a healthy session with tunnels still wins.
    async fn take_over_conflict(&self, conflict: &AgentConflict) -> bool {
        let old = {
            let map = self.controls.lock().await;
            map.get(&conflict.session_id)
                .map(|entry| Arc::clone(&entry.control))
        };

        let (present, closed, heartbeat_stale, tunnel_count) = match old.as_ref() {
            Some(old) => {
                let closed = old.is_closed();
                let heartbeat_stale = old.is_heartbeat_stale();
                let tunnels = if closed || heartbeat_stale {
                    Some(0)
                } else {
                    old.try_tunnel_count(CONFLICT_PROBE_TIMEOUT).await
                };
                (true, closed, heartbeat_stale, tunnels)
            }
            None => (false, false, false, None),
        };

        if resolve_conflict(present, closed, heartbeat_stale, tunnel_count)
            == ConflictResolution::Reject
        {
            return false;
        }

        tracing::warn!(
            session_id = %conflict.session_id,
            generation = conflict.generation,
            agent_id = %conflict.agent_id,
            old_tunnels = tunnel_count.unwrap_or(conflict.tunnel_count),
            control_present = present,
            control_closed = closed,
            heartbeat_stale,
            "taking over stale agent session"
        );

        if let Some(old) = old {
            let removed = timeout(
                TAKEOVER_REMOVE_TIMEOUT,
                remove_if_current(&self.controls, &old.session_id, &old),
            )
            .await;
            if !matches!(removed, Ok(true)) {
                let mut map = self.controls.lock().await;
                if map
                    .get(&old.session_id)
                    .map(|entry| Arc::ptr_eq(&entry.control, &old))
                    .unwrap_or(false)
                {
                    map.remove(&old.session_id);
                }
            }

            if timeout(TAKEOVER_SHUTDOWN_TIMEOUT, old.shutdown())
                .await
                .is_err()
            {
                tracing::warn!(
                    session_id = %old.session_id,
                    generation = old.generation,
                    "old control shutdown timed out during takeover"
                );
            }
            old.abandon_tunnels_now();
        }

        // Free any global tunnel names and ports still owned by the stale
        // generation. Without this a restarted client can log in but every
        // tunnel registration fails with "already registered".
        self.release_stale_tunnel_slots(&conflict.session_id, conflict.generation);

        self.agents.force_release(
            &conflict.session_id,
            Some(conflict.generation),
            tunnel_count.unwrap_or(conflict.tunnel_count),
        );
        true
    }

    pub(crate) async fn register_data_conn(
        self: Arc<Self>,
        stream: DynStream,
        nw: NewDataConn,
    ) -> Result<()> {
        if nw.session_id.trim().is_empty() {
            return Err(anyhow!("empty session_id for data connection"));
        }
        if let Err(reason) = auth::verify_auth_digest(
            &self.cfg.auth.token,
            &nw.auth_digest,
            nw.timestamp,
            auth::unix_now_secs(),
            None,
        ) {
            tracing::warn!(
                %reason,
                session_id = %nw.session_id,
                "data connection authentication failed"
            );
            return Err(anyhow!(
                "data connection authentication failed for session_id={}",
                nw.session_id
            ));
        }

        match session_table::lookup_accepting(&self.controls, &nw.session_id).await {
            Some(c) => {
                c.push_data_conn(stream).await;
                Ok(())
            }
            None => Err(anyhow!(
                "no accepting control for data connection session_id={}",
                nw.session_id
            )),
        }
    }
}

async fn reject_login(mut stream: DynStream, error: &str) -> Result<()> {
    let _ = msg::write_msg(
        &mut stream,
        &Message::LoginResp(LoginResp {
            version: VERSION.into(),
            session_id: String::new(),
            error: error.into(),
        }),
    )
    .await;
    Err(anyhow!(error.to_string()))
}

async fn reject_control(
    controls: &tokio::sync::Mutex<session_table::SessionMap>,
    session_id: &str,
    control: &Arc<Control>,
    error: String,
) -> Result<()> {
    let _ = timeout(
        TAKEOVER_REMOVE_TIMEOUT,
        remove_if_current(controls, session_id, control),
    )
    .await;
    let _ = control.send_login_err(VERSION, &error).await;
    control.shutdown().await;
    Err(anyhow!(error))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictResolution {
    Reject,
    Takeover,
}

fn resolve_conflict(
    control_present: bool,
    closed: bool,
    heartbeat_stale: bool,
    tunnel_count: Option<usize>,
) -> ConflictResolution {
    if !control_present || closed || heartbeat_stale {
        return ConflictResolution::Takeover;
    }
    match tunnel_count {
        Some(0) => ConflictResolution::Takeover,
        Some(_) => ConflictResolution::Reject,
        // We could not inspect the old control; assume it is still serving.
        None => ConflictResolution::Reject,
    }
}

fn short_session_id() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    hex[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use super::{resolve_conflict, ConflictResolution};

    #[test]
    fn missing_control_is_taken_over() {
        assert_eq!(
            resolve_conflict(false, false, false, None),
            ConflictResolution::Takeover
        );
    }

    #[test]
    fn closed_or_stale_control_is_taken_over() {
        assert_eq!(
            resolve_conflict(true, true, false, Some(2)),
            ConflictResolution::Takeover
        );
        assert_eq!(
            resolve_conflict(true, false, true, Some(2)),
            ConflictResolution::Takeover
        );
    }

    #[test]
    fn zero_tunnel_control_is_taken_over() {
        assert_eq!(
            resolve_conflict(true, false, false, Some(0)),
            ConflictResolution::Takeover
        );
    }

    #[test]
    fn healthy_control_with_tunnels_is_kept() {
        assert_eq!(
            resolve_conflict(true, false, false, Some(2)),
            ConflictResolution::Reject
        );
    }

    #[test]
    fn unknown_tunnel_count_is_kept() {
        assert_eq!(
            resolve_conflict(true, false, false, None),
            ConflictResolution::Reject
        );
    }
}
