mod data_pool;
mod register;

use crate::access::AccessPolicy;
use crate::metrics::{MemMetrics, ServerMetrics};
use crate::tunnel::{
    DetachedTunnel, HttpGw, HttpsGw, PortTable, TunnelManager, TunnelOwner, TunnelRegistry,
};
use anyhow::Result;
use orbien_core::config::ServerConfig;
use orbien_core::msg::{self, KickOut, Message, Ping, Pong};
use orbien_core::transport::DynStream;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::sync::{mpsc, watch, Mutex, Notify};
use tokio::task::JoinSet;
use tokio::time::sleep;

type CtrlRead = ReadHalf<DynStream>;
type CtrlWrite = WriteHalf<DynStream>;

pub struct Control {
    pub session_id: String,
    pub generation: u64,
    pub user: String,
    pub agent_id: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub client_ip: String,
    pub connected_at: Instant,
    cfg: ServerConfig,
    reader: Mutex<CtrlRead>,
    writer: Mutex<CtrlWrite>,
    data_tx: mpsc::Sender<DynStream>,
    data_rx: Mutex<mpsc::Receiver<DynStream>>,
    data_notify: Notify,
    shutdown_notify: Notify,
    tunnels: Mutex<TunnelManager>,
    tunnel_registry: Arc<TunnelRegistry>,
    tcp_ports: Arc<PortTable>,
    udp_ports: Arc<PortTable>,
    bg_tasks: Mutex<JoinSet<()>>,
    closed: AtomicBool,
    cleaning: AtomicBool,
    finished: watch::Sender<bool>,
    activated: AtomicBool,
    pool_count: usize,
    http_gw: Option<Arc<HttpGw>>,
    https_gw: Option<Arc<HttpsGw>>,
    access: Arc<AccessPolicy>,
    pub metrics: Arc<MemMetrics>,
    last_ping_unix: AtomicI64,
}

impl Control {
    pub fn new(
        session_id: String,
        generation: u64,
        stream: DynStream,
        cfg: ServerConfig,
        pool_count: usize,
        http_gw: Option<Arc<HttpGw>>,
        https_gw: Option<Arc<HttpsGw>>,
        access: Arc<AccessPolicy>,
        user: String,
        agent_id: String,
        hostname: String,
        os: String,
        arch: String,
        version: String,
        client_ip: String,
        metrics: Arc<MemMetrics>,
        tunnel_registry: Arc<TunnelRegistry>,
        tcp_ports: Arc<PortTable>,
        udp_ports: Arc<PortTable>,
    ) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        let (finished, _) = watch::channel(false);
        let (data_tx, data_rx) = mpsc::channel(64);
        Self {
            session_id,
            generation,
            user,
            agent_id,
            hostname,
            os,
            arch,
            version,
            client_ip,
            connected_at: Instant::now(),
            cfg,
            reader: Mutex::new(reader),
            writer: Mutex::new(writer),
            data_tx,
            data_rx: Mutex::new(data_rx),
            data_notify: Notify::new(),
            shutdown_notify: Notify::new(),
            tunnels: Mutex::new(TunnelManager::new()),
            tunnel_registry,
            tcp_ports,
            udp_ports,
            bg_tasks: Mutex::new(JoinSet::new()),
            closed: AtomicBool::new(false),
            cleaning: AtomicBool::new(false),
            finished,
            activated: AtomicBool::new(false),
            pool_count: pool_count.max(1),
            http_gw,
            https_gw,
            access,
            metrics,
            last_ping_unix: AtomicI64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            ),
        }
    }

    pub fn owner(&self) -> TunnelOwner {
        TunnelOwner {
            session_id: self.session_id.clone(),
            generation: self.generation,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn is_accepting_data(&self) -> bool {
        self.activated.load(Ordering::Acquire) && !self.is_closed()
    }

    pub async fn tunnel_summaries(&self) -> Vec<crate::tunnel::TunnelSummary> {
        self.tunnels.lock().await.summaries()
    }

    pub async fn tunnel_count(&self) -> usize {
        self.tunnels.lock().await.len()
    }

    pub async fn wait_finished(&self) {
        let mut rx = self.finished.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    fn mark_finished(&self) {
        let _ = self.finished.send(true);
    }

    pub async fn send_login_ok(&self, version: &str) -> Result<()> {
        let mut writer = self.writer.lock().await;
        msg::write_msg(
            &mut *writer,
            &Message::LoginResp(orbien_core::msg::LoginResp {
                version: version.into(),
                session_id: self.session_id.clone(),
                error: String::new(),
            }),
        )
        .await?;
        drop(writer);
        self.activated.store(true, Ordering::Release);
        Ok(())
    }

    pub async fn send_login_err(&self, version: &str, error: &str) -> Result<()> {
        let mut writer = self.writer.lock().await;
        msg::write_msg(
            &mut *writer,
            &Message::LoginResp(orbien_core::msg::LoginResp {
                version: version.into(),
                session_id: String::new(),
                error: error.into(),
            }),
        )
        .await?;
        Ok(())
    }

    pub(super) fn release_global_slot(&self, name: &str, detached: &DetachedTunnel) {
        if let Some(port) = detached.remote_port {
            match detached.tunnel_type {
                "tcp" => self.tcp_ports.release(port, name),
                "udp" => self.udp_ports.release(port, name),
                _ => {}
            }
        }
        self.tunnel_registry.remove_if_owner(name, &self.owner());
    }

    pub(super) async fn detach_tunnel(&self, name: &str) -> Option<&'static str> {
        let detached = {
            let mut tm = self.tunnels.lock().await;
            tm.remove(name).await
        }?;
        let ty = detached.tunnel_type;
        self.release_global_slot(name, &detached);
        Some(ty)
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        for _ in 0..self.pool_count {
            if self.closed.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.request_data_conn().await?;
        }

        {
            let timeout = self.effective_ping_timeout();
            if timeout > 0 {
                let this = Arc::clone(&self);
                self.spawn_bg(async move {
                    loop {
                        if this.closed.load(Ordering::SeqCst) {
                            break;
                        }
                        sleep(Duration::from_secs(1)).await;
                        let last = this.last_ping_unix.load(Ordering::Relaxed);
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        if last > 0 && now.saturating_sub(last) > timeout {
                            tracing::warn!(
                                session_id = %this.session_id,
                                generation = this.generation,
                                timeout_secs = timeout,
                                "heartbeat timeout"
                            );
                            this.signal_close();
                            break;
                        }
                    }
                })
                .await;
            }
        }

        loop {
            if self.closed.load(Ordering::SeqCst) {
                break;
            }
            self.reap_bg_tasks().await;
            let msg = tokio::select! {
                _ = self.shutdown_notify.notified() => {
                    break;
                }
                msg = async {
                    let mut reader = self.reader.lock().await;
                    msg::read_msg(&mut *reader).await
                } => {
                    match msg {
                        Ok(m) => m,
                        Err(e) => {
                            if !self.closed.load(Ordering::SeqCst) {
                                tracing::debug!(error = %e, "control read ended");
                            }
                            break;
                        }
                    }
                }
            };

            match msg {
                Message::NewTunnel(np) => self.handle_new_tunnel(np).await?,
                Message::CloseTunnel(cp) => self.handle_close_tunnel(cp).await?,
                Message::Ping(p) => self.handle_ping(p).await?,
                other => {
                    tracing::warn!(ty = other.type_byte(), "ignored control message");
                }
            }
        }
        Ok(())
    }

    pub(super) fn signal_close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.shutdown_notify.notify_waiters();
        self.data_notify.notify_waiters();
    }

    pub(super) async fn spawn_bg<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut bg = self.bg_tasks.lock().await;
        if self.cleaning.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            return;
        }
        reap_join_set(&mut bg);
        bg.spawn(fut);
    }

    async fn reap_bg_tasks(&self) {
        let mut bg = self.bg_tasks.lock().await;
        reap_join_set(&mut bg);
    }

    pub async fn shutdown(&self) {
        self.signal_close();
        if self.cleaning.swap(true, Ordering::SeqCst) {
            self.wait_finished().await;
            return;
        }
        {
            let mut tm = self.tunnels.lock().await;
            for (name, detached) in tm.close_all().await {
                self.release_global_slot(&name, &detached);
                self.metrics.close_tunnel(&name, detached.tunnel_type);
            }
        }
        {
            let mut writer = self.writer.lock().await;
            let _ = writer.shutdown().await;
        }
        {
            let mut bg = self.bg_tasks.lock().await;
            bg.abort_all();
            while bg.join_next().await.is_some() {}
        }
        self.mark_finished();
    }

    pub async fn kick(&self, reason: impl Into<String>) {
        let reason = reason.into();
        {
            let mut writer = self.writer.lock().await;
            let _ = msg::write_msg(
                &mut *writer,
                &Message::KickOut(KickOut {
                    reason: reason.clone(),
                }),
            )
            .await;
        }
        tracing::info!(
            session_id = %self.session_id,
            generation = self.generation,
            %reason,
            "kicking client"
        );
        self.shutdown().await;
    }

    fn effective_ping_timeout(&self) -> i64 {
        let hb_to = self.cfg.transport.heartbeat_timeout;
        if hb_to > 0 {
            return hb_to;
        }
        if self.cfg.transport.tcp_mux {
            let mux_ka = self.cfg.transport.mux_keepalive_secs;
            if mux_ka > 0 {
                return mux_ka.saturating_mul(3);
            }
        }
        -1
    }

    async fn handle_ping(&self, _p: Ping) -> Result<()> {
        self.last_ping_unix.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            Ordering::Relaxed,
        );
        let mut writer = self.writer.lock().await;
        msg::write_msg(&mut *writer, &Message::Pong(Pong::default())).await?;
        Ok(())
    }
}

fn reap_join_set(bg: &mut JoinSet<()>) {
    while let Some(res) = bg.try_join_next() {
        if let Err(e) = res {
            if !e.is_cancelled() {
                tracing::debug!(error = %e, "bg task join error");
            }
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        if *self.finished.borrow() {
            return;
        }
        if let Ok(mut tm) = self.tunnels.try_lock() {
            for (name, detached) in tm.abandon_all() {
                self.release_global_slot(&name, &detached);
            }
        }
        self.mark_finished();
    }
}
