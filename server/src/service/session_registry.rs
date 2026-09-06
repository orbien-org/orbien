use super::agent_registry::{
    sanitize_wire_id, AgentOnlineSpec, AgentRegisterError, MAX_AGENT_ID_LEN, MAX_SESSION_ID_LEN,
    MAX_USER_LEN,
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
use uuid::Uuid;

impl Service {
    pub(crate) async fn register_control(
        self: Arc<Self>,
        stream: DynStream,
        login: Login,
        peer: SocketAddr,
    ) -> Result<()> {
        if !auth::verify_login(&self.cfg.auth.token, &login.auth_digest, login.timestamp) {
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
            old.wait_finished().await;
        }

        match self.agents.try_online(AgentOnlineSpec {
            user: user.clone(),
            agent_id: agent_id.clone(),
            session_id: session_id.clone(),
            generation,
            hostname: control.hostname.clone(),
            os: control.os.clone(),
            arch: control.arch.clone(),
            client_ip: control.client_ip.clone(),
            version: control.version.clone(),
        }) {
            Ok(_) => {}
            Err(AgentRegisterError::Conflict) => {
                drop(session_guard);
                let _ = remove_if_current(&self.controls, &session_id, &control).await;
                let err = format!("agent_id [{agent_id}] for user [{user}] is already online");
                let _ = control.send_login_err(VERSION, &err).await;
                control.shutdown().await;
                return Err(anyhow!(err));
            }
        }

        if let Err(e) = control.send_login_ok(VERSION).await {
            self.agents.release(&session_id, generation, 0);
            drop(session_guard);
            let _ = remove_if_current(&self.controls, &session_id, &control).await;
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
        control.shutdown().await;
        metrics.close_client();

        let tunnel_count = control.tunnel_count().await;
        let _ = remove_if_current(&controls, &rid, &control).await;
        agents.release(&rid, generation, tunnel_count);

        result
    }

    pub(crate) async fn register_data_conn(
        self: Arc<Self>,
        stream: DynStream,
        nw: NewDataConn,
    ) -> Result<()> {
        if nw.session_id.trim().is_empty() {
            return Err(anyhow!("empty session_id for data conn"));
        }
        if !auth::verify_auth_digest(&self.cfg.auth.token, &nw.auth_digest, nw.timestamp) {
            return Err(anyhow!(
                "data conn auth failed for session_id={}",
                nw.session_id
            ));
        }

        match session_table::lookup_accepting(&self.controls, &nw.session_id).await {
            Some(c) => {
                c.push_data_conn(stream).await;
                Ok(())
            }
            None => Err(anyhow!(
                "no accepting control for data conn session_id={}",
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

fn short_session_id() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    hex[..16].to_owned()
}
