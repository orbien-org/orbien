use crate::connector::{build_connector, Connector};
use crate::reload::{ReloadLevel, ReloadOutcome, TunnelChanges};
use crate::tunnel::TunnelManager;
use anyhow::{anyhow, Result};
use orbien_core::auth;
use orbien_core::config::{ClientConfig, TunnelConfig};
use orbien_core::msg::{self, CloseTunnel, Login, Message, NewDataConn, NewTunnel, Ping};
use orbien_core::transport::DynStream;
use orbien_core::VERSION;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinSet;
use tokio::time::{interval, sleep};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct LoginRejected {
    pub reason: String,
}

impl fmt::Display for LoginRejected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "login rejected: {}", self.reason)
    }
}

impl std::error::Error for LoginRejected {}

#[derive(Debug)]
pub enum SessionEnd {
    Disconnected { session_id: String },
    Kicked { session_id: String, reason: String },
}

type CtrlRead = ReadHalf<DynStream>;
type CtrlWrite = WriteHalf<DynStream>;
type OnTunnelRemote = Arc<dyn Fn(String, String) + Send + Sync>;
type OnTunnelRemoved = Arc<dyn Fn(String) + Send + Sync>;

pub struct ActiveSession {
    pub control: Arc<Control>,
    pub(crate) done: oneshot::Receiver<Result<SessionEnd>>,
}

pub struct Control {
    cfg: Arc<RwLock<ClientConfig>>,
    session_id: String,
    reader: Mutex<CtrlRead>,
    writer: Mutex<CtrlWrite>,
    tunnels: TunnelManager,
    connector: Arc<dyn Connector>,
    cancel: CancellationToken,
    data_tasks: Mutex<JoinSet<()>>,
    on_tunnel_remote: OnTunnelRemote,
    on_tunnel_removed: OnTunnelRemoved,
    last_pong_unix: AtomicI64,
    register_watch: StdMutex<RegisterWatch>,
}

struct RegisterWatch {
    pending: HashSet<String>,
    errors: HashMap<String, String>,
}

impl RegisterWatch {
    fn begin(&mut self, name: &str) {
        self.errors.remove(name);
        self.pending.insert(name.to_string());
    }

    fn finish(&mut self, name: &str, error: Option<String>) {
        self.pending.remove(name);
        if let Some(err) = error {
            self.errors.insert(name.to_string(), err);
        }
    }
}

impl Control {
    pub async fn open_session(
        cfg: Arc<RwLock<ClientConfig>>,
        previous_session_id: String,
        parent_cancel: CancellationToken,
        on_connected: impl FnOnce(),
        on_tunnel_remote: OnTunnelRemote,
        on_tunnel_removed: OnTunnelRemoved,
    ) -> Result<ActiveSession> {
        let cfg_snapshot = cfg.read().await.clone();
        let session_cancel = parent_cancel.child_token();
        let connector = build_connector(&cfg_snapshot).await?;
        let mut stream = connector.open().await?;
        tracing::info!(
            endpoint = %cfg_snapshot.server_endpoint(),
            protocol = %cfg_snapshot.transport.protocol,
            tcp_mux = cfg_snapshot.uses_yamux(),
            "control stream opened"
        );

        let timestamp = now_secs();
        let auth_digest = auth::compute_auth_digest(&cfg_snapshot.auth.token, timestamp);
        let login = Login {
            version: VERSION.into(),
            hostname: hostname(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            user: cfg_snapshot.user.clone(),
            agent_id: cfg_snapshot.agent_id.clone(),
            auth_digest,
            timestamp,
            session_id: previous_session_id,
            pool_count: cfg_snapshot.transport.pool_count,
        };
        tracing::info!(
            hostname = %login.hostname,
            os = %login.os,
            arch = %login.arch,
            user = %login.user,
            agent_id = %login.agent_id,
            "login identity"
        );

        msg::write_msg(&mut stream, &Message::Login(login)).await?;
        let resp = match msg::read_msg(&mut stream).await? {
            Message::LoginResp(r) => r,
            other => {
                return Err(anyhow!(
                    "expected LoginResp, got type {}",
                    other.type_byte()
                ))
            }
        };

        if !resp.error.is_empty() {
            return Err(LoginRejected { reason: resp.error }.into());
        }

        tracing::info!(session_id = %resp.session_id, "login ok");

        let (reader, writer) = tokio::io::split(stream);
        let ctl = Arc::new(Control {
            cfg: Arc::clone(&cfg),
            session_id: resp.session_id.clone(),
            reader: Mutex::new(reader),
            writer: Mutex::new(writer),
            tunnels: TunnelManager::from_config(&cfg_snapshot)?,
            connector,
            cancel: session_cancel.clone(),
            data_tasks: Mutex::new(JoinSet::new()),
            on_tunnel_remote,
            on_tunnel_removed,
            last_pong_unix: AtomicI64::new(now_secs()),
            register_watch: StdMutex::new(RegisterWatch {
                pending: HashSet::new(),
                errors: HashMap::new(),
            }),
        });

        ctl.register_all_tunnels().await?;
        on_connected();

        let (done_tx, done_rx) = oneshot::channel();
        let session_id = resp.session_id.clone();
        let runner = Arc::clone(&ctl);
        let hb_cancel = session_cancel.clone();
        tokio::spawn(async move {
            let hb = Arc::clone(&runner);
            let heartbeat = tokio::spawn(async move {
                tokio::select! {
                    _ = hb_cancel.cancelled() => {}
                    _ = hb.heartbeat_loop() => {}
                }
            });

            let to = Arc::clone(&runner);
            let to_cancel = session_cancel.clone();
            let timeout_watch = tokio::spawn(async move {
                tokio::select! {
                    _ = to_cancel.cancelled() => {}
                    _ = to.heartbeat_timeout_loop() => {}
                }
            });

            let result = runner.clone().reader_loop().await;
            runner.shutdown().await;
            heartbeat.abort();
            timeout_watch.abort();
            let _ = heartbeat.await;
            let _ = timeout_watch.await;

            let end = match result {
                Ok(ReaderEnd::Kicked(reason)) => Ok(SessionEnd::Kicked {
                    session_id: session_id.clone(),
                    reason,
                }),
                Ok(ReaderEnd::Closed) => Ok(SessionEnd::Disconnected { session_id }),
                Err(e) => Err(e),
            };
            let _ = done_tx.send(end);
        });

        Ok(ActiveSession {
            control: ctl,
            done: done_rx,
        })
    }

    pub fn request_disconnect(&self) {
        self.cancel.cancel();
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub async fn close_tunnel(&self, name: &str) -> Result<()> {
        let msg = Message::CloseTunnel(CloseTunnel {
            tunnel_name: name.into(),
        });
        let mut writer = self.writer.lock().await;
        msg::write_msg(&mut *writer, &msg).await?;
        drop(writer);
        self.tunnels.remove(name);
        (self.on_tunnel_removed)(name.into());
        tracing::info!(name = %name, "tunnel closed");
        Ok(())
    }

    pub async fn register_tunnel(&self, tunnel: &TunnelConfig) -> Result<()> {
        validate_tunnel(tunnel)?;
        let msg = build_new_tunnel_message(tunnel)?;
        self.tunnels.upsert(tunnel)?;
        self.register_watch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .begin(&tunnel.name);
        let mut writer = self.writer.lock().await;
        msg::write_msg(&mut *writer, &msg).await?;
        tracing::info!(name = %tunnel.name, protocol = %tunnel.protocol, "sent NewTunnel");
        Ok(())
    }

    pub async fn close_all_tunnels(&self) {
        for name in self.tunnels.tunnel_names() {
            if let Err(e) = self.close_tunnel(&name).await {
                tracing::warn!(name = %name, error = %e, "failed to close tunnel");
            }
        }
    }

    pub async fn apply_tunnel_changes(&self, changes: &TunnelChanges) -> ReloadOutcome {
        let mut outcome = ReloadOutcome {
            level: ReloadLevel::TunnelsOnly,
            ..Default::default()
        };

        let mut stop_names = changes.removed.clone();
        for tunnel in &changes.updated {
            if !stop_names.iter().any(|n| n == &tunnel.name) {
                stop_names.push(tunnel.name.clone());
            }
        }
        stop_names.sort();
        stop_names.dedup();

        for name in &stop_names {
            match self.close_tunnel(name).await {
                Ok(()) => {
                    if changes.removed.iter().any(|n| n == name) {
                        outcome.removed.push(name.clone());
                    }
                }
                Err(e) => outcome.failed.push((name.clone(), e.to_string())),
            }
        }

        let mut started = Vec::new();
        for tunnel in changes.updated.iter().chain(changes.added.iter()) {
            if outcome.failed.iter().any(|(n, _)| n == &tunnel.name) {
                continue;
            }
            let is_added = changes.added.iter().any(|t| t.name == tunnel.name);
            match self.register_tunnel(tunnel).await {
                Ok(()) => {
                    started.push(tunnel.name.clone());
                    if is_added {
                        outcome.added.push(tunnel.name.clone());
                    } else {
                        outcome.updated.push(tunnel.name.clone());
                    }
                }
                Err(e) => outcome.failed.push((tunnel.name.clone(), e.to_string())),
            }
        }

        if !started.is_empty() {
            let register_errors = self
                .collect_register_errors(&started, Duration::from_secs(5))
                .await;
            for (name, err) in register_errors {
                outcome.added.retain(|n| n != &name);
                outcome.updated.retain(|n| n != &name);
                if !outcome.failed.iter().any(|(n, _)| n == &name) {
                    outcome.failed.push((name, err));
                }
            }
        }

        outcome
    }

    async fn collect_register_errors(
        &self,
        names: &[String],
        timeout: Duration,
    ) -> Vec<(String, String)> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let watch = self
                    .register_watch
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if !names.iter().any(|n| watch.pending.contains(n)) {
                    return names
                        .iter()
                        .filter_map(|n| watch.errors.get(n).map(|e| (n.clone(), e.clone())))
                        .collect();
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let watch = self
                    .register_watch
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                return names
                    .iter()
                    .filter_map(|n| {
                        if watch.pending.contains(n) {
                            Some((n.clone(), "tunnel start timed out".into()))
                        } else {
                            watch.errors.get(n).map(|e| (n.clone(), e.clone()))
                        }
                    })
                    .collect();
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn shutdown(&self) {
        self.cancel.cancel();
        {
            let mut writer = self.writer.lock().await;
            let _ = writer.shutdown().await;
        }
        let mut tasks = self.data_tasks.lock().await;
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    async fn register_all_tunnels(&self) -> Result<()> {
        let tunnels = self.cfg.read().await.tunnels.clone();
        for p in &tunnels {
            validate_tunnel(p)?;
            let msg = build_new_tunnel_message(p)?;
            self.tunnels.upsert(p)?;
            let mut writer = self.writer.lock().await;
            msg::write_msg(&mut *writer, &msg).await?;
            tracing::info!(
                name = %p.name,
                protocol = %p.protocol,
                "sent NewTunnel"
            );
        }
        Ok(())
    }

    async fn reader_loop(self: Arc<Self>) -> Result<ReaderEnd> {
        let mut data_tasks = {
            let mut slot = self.data_tasks.lock().await;
            std::mem::take(&mut *slot)
        };

        let end = self.reader_loop_inner(&mut data_tasks).await;

        {
            let mut slot = self.data_tasks.lock().await;
            *slot = data_tasks;
        }
        end
    }

    async fn reader_loop_inner(
        self: &Arc<Self>,
        data_tasks: &mut JoinSet<()>,
    ) -> Result<ReaderEnd> {
        loop {
            if self.cancel.is_cancelled() {
                return Ok(ReaderEnd::Closed);
            }

            let msg = tokio::select! {
                _ = self.cancel.cancelled() => {
                    return Ok(ReaderEnd::Closed);
                }
                joined = data_tasks.join_next(), if !data_tasks.is_empty() => {
                    if let Some(Err(e)) = joined {
                        if !e.is_cancelled() {
                            tracing::debug!(error = %e, "data task join error");
                        }
                    }
                    continue;
                }
                msg = async {
                    let mut reader = self.reader.lock().await;
                    msg::read_msg(&mut *reader).await
                } => {
                    match msg {
                        Ok(m) => m,
                        Err(_) => return Ok(ReaderEnd::Closed),
                    }
                }
            };

            match msg {
                Message::KickOut(k) => {
                    tracing::warn!(reason = %k.reason, "kicked by server");
                    return Ok(ReaderEnd::Kicked(k.reason));
                }
                Message::ReqDataConn(_) => {
                    while data_tasks.try_join_next().is_some() {}
                    let ctl = Arc::clone(self);
                    let cancel = self.cancel.clone();
                    data_tasks.spawn(async move {
                        tokio::select! {
                            _ = cancel.cancelled() => {}
                            res = ctl.handle_req_data_conn() => {
                                if let Err(e) = res {
                                    tracing::error!(error = %e, "data conn failed");
                                }
                            }
                        }
                    });
                }
                Message::NewTunnelResp(resp) => {
                    if resp.error.is_empty() {
                        self.register_watch
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .finish(&resp.tunnel_name, None);
                        let server = self.cfg.read().await.server.clone();
                        let remote = normalize_remote_addr(&server, &resp.remote_addr);
                        tracing::info!(
                            name = %resp.tunnel_name,
                            remote = %remote,
                            "tunnel started"
                        );
                        (self.on_tunnel_remote)(resp.tunnel_name.clone(), remote);
                    } else {
                        self.register_watch
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .finish(&resp.tunnel_name, Some(resp.error.clone()));
                        tracing::error!(
                            name = %resp.tunnel_name,
                            error = %resp.error,
                            "tunnel start failed"
                        );
                    }
                }
                Message::Pong(_) => {
                    self.last_pong_unix.store(now_secs(), Ordering::Relaxed);
                    tracing::trace!("pong");
                }
                other => {
                    tracing::warn!(ty = other.type_byte(), "ignored message");
                }
            }
        }
    }

    async fn heartbeat_loop(self: Arc<Self>) {
        let secs = self.effective_ping_interval();
        if secs <= 0 {
            tracing::debug!("app heartbeat disabled");
            std::future::pending::<()>().await;
            return;
        }
        let mut tick = interval(Duration::from_secs(secs as u64));
        tick.tick().await;
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            tick.tick().await;
            let timestamp = now_secs();
            let token = self.cfg.read().await.auth.token.clone();
            let ping = Ping {
                auth_digest: auth::compute_auth_digest(&token, timestamp),
                timestamp,
            };
            let mut writer = self.writer.lock().await;
            if msg::write_msg(&mut *writer, &Message::Ping(ping))
                .await
                .is_err()
            {
                break;
            }
        }
    }

    async fn heartbeat_timeout_loop(self: Arc<Self>) {
        let timeout = self.effective_pong_timeout();
        if timeout <= 0 {
            std::future::pending::<()>().await;
            return;
        }
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            sleep(Duration::from_secs(1)).await;
            let last = self.last_pong_unix.load(Ordering::Relaxed);
            let now = now_secs();
            if last > 0 && now.saturating_sub(last) > timeout {
                tracing::warn!(timeout_secs = timeout, "heartbeat timeout");
                self.cancel.cancel();
                break;
            }
        }
    }

    fn effective_ping_interval(&self) -> i64 {
        let transport = self
            .cfg
            .try_read()
            .map(|c| c.transport.clone())
            .unwrap_or_default();
        let hb = transport.heartbeat_interval;
        if hb > 0 {
            return hb;
        }
        if transport.tcp_mux {
            let mux_ka = transport.mux_keepalive_secs;
            if mux_ka > 0 {
                return mux_ka;
            }
        }
        -1
    }

    fn effective_pong_timeout(&self) -> i64 {
        let transport = self
            .cfg
            .try_read()
            .map(|c| c.transport.clone())
            .unwrap_or_default();
        let hb_to = transport.heartbeat_timeout;
        if hb_to > 0 {
            return hb_to;
        }
        if transport.heartbeat_interval <= 0 && transport.tcp_mux {
            let mux_ka = transport.mux_keepalive_secs;
            if mux_ka > 0 {
                return mux_ka.saturating_mul(3);
            }
        }
        -1
    }

    async fn handle_req_data_conn(self: Arc<Self>) -> Result<()> {
        let mut data = self.connector.open().await?;

        let timestamp = now_secs();
        let (session_id, token) = {
            let cfg = self.cfg.read().await;
            (self.session_id.clone(), cfg.auth.token.clone())
        };
        msg::write_msg(
            &mut data,
            &Message::NewDataConn(NewDataConn {
                session_id,
                auth_digest: auth::compute_auth_digest(&token, timestamp),
                timestamp,
            }),
        )
        .await?;

        let start = tokio::select! {
            _ = self.cancel.cancelled() => {
                return Ok(());
            }
            msg = msg::read_msg(&mut data) => {
                match msg? {
                    Message::StartDataConn(s) => s,
                    other => {
                        return Err(anyhow!("expected StartDataConn, got {}", other.type_byte()))
                    }
                }
            }
        };

        if !start.error.is_empty() {
            return Err(anyhow!("StartDataConn error: {}", start.error));
        }

        self.tunnels.handle_data_conn(&start, data).await
    }
}

enum ReaderEnd {
    Closed,
    Kicked(String),
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn hostname() -> String {
    if let Ok(name) = hostname::get() {
        let s = name.to_string_lossy().trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }

    ["HOSTNAME", "COMPUTERNAME", "HOST"]
        .into_iter()
        .find_map(|k| std::env::var(k).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn omit_client_side(side: &str) -> String {
    match side.trim().to_ascii_lowercase().as_str() {
        "" | "client" => String::new(),
        other => other.to_string(),
    }
}

fn normalize_remote_addr(server_addr: &str, remote_addr: &str) -> String {
    let remote = remote_addr.trim();
    if remote.is_empty() {
        return String::new();
    }
    if let Some(port) = remote.strip_prefix(':') {
        let host = server_addr.trim();
        let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
        if !host.is_empty() && !port.is_empty() && !host.contains(':') {
            return format!("{host}:{port}");
        }
        if !host.is_empty() && !port.is_empty() {
            return format!("{host}:{port}");
        }
    }
    remote.to_string()
}

fn validate_tunnel(p: &TunnelConfig) -> Result<()> {
    let (_local_ip, local_port) = p.service_host_port()?;
    if p.requires_local_service() && local_port == 0 {
        return Err(anyhow!(
            "tunnel `{}` requires service = \"host:port\" (local backend)",
            p.name
        ));
    }
    if p.remote_port == 0 && matches!(p.protocol.as_str(), "tcp" | "udp") {
        return Err(anyhow!(
            "tunnel `{}` type {} requires remotePort > 0",
            p.name,
            p.protocol
        ));
    }
    match p.protocol.as_str() {
        "tcp" | "udp" | "http" | "https" => Ok(()),
        other => Err(anyhow!(
            "tunnel `{}` unsupported protocol {}",
            p.name,
            other
        )),
    }
}

fn build_new_tunnel_message(p: &TunnelConfig) -> Result<Message> {
    let (local_ip, local_port) = p.service_host_port()?;
    let msg = match p.protocol.as_str() {
        "tcp" => Message::NewTunnel(new_tunnel_base(
            &p.name,
            "tcp",
            p.remote_port as i32,
            &local_ip,
            local_port,
            &p.transport,
            |_| {},
        )),
        "udp" => Message::NewTunnel(new_tunnel_base(
            &p.name,
            "udp",
            p.remote_port as i32,
            &local_ip,
            local_port,
            &p.transport,
            |_| {},
        )),
        "http" => Message::NewTunnel(new_tunnel_base(
            &p.name,
            "http",
            0,
            &local_ip,
            local_port,
            &p.transport,
            |np| {
                np.domains = p.domains.clone();
                np.locations = p.locations.clone();
                np.basic_auth_user = p.basic_auth_user.clone();
                np.basic_auth_password = p.basic_auth_password.clone();
                np.host_header_rewrite = p.host_header_rewrite.clone();
            },
        )),
        "https" => Message::NewTunnel(new_tunnel_base(
            &p.name,
            "https",
            0,
            &local_ip,
            local_port,
            &p.transport,
            |np| {
                np.domains = p.domains.clone();
            },
        )),
        other => {
            return Err(anyhow!(
                "tunnel `{}` unsupported protocol {}",
                p.name,
                other
            ));
        }
    };
    Ok(msg)
}

fn new_tunnel_base(
    name: &str,
    protocol: &str,
    remote_port: i32,
    local_ip: &str,
    local_port: u16,
    transport: &orbien_core::config::TunnelTransportConfig,
    extra: impl FnOnce(&mut NewTunnel),
) -> NewTunnel {
    let mut np = NewTunnel {
        tunnel_name: name.into(),
        protocol: protocol.into(),
        remote_port,
        local_ip: local_ip.into(),
        local_port: i32::from(local_port),
        domains: Vec::new(),
        locations: Vec::new(),
        basic_auth_user: String::new(),
        basic_auth_password: String::new(),
        host_header_rewrite: String::new(),
        headers: Default::default(),
        response_headers: Default::default(),
        bandwidth: transport.bandwidth,
        bandwidth_limit_side: omit_client_side(&transport.bandwidth_limit_side),
    };
    extra(&mut np);
    np
}
