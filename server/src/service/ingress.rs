use super::Service;
use anyhow::{anyhow, Result};
use orbien_core::msg::{self, Message};
use orbien_core::transport::{self, boxed_stream, DynStream};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

impl Service {
    pub(super) async fn run_tcp(self: Arc<Self>, listener: TcpListener) -> Result<()> {
        loop {
            let (stream, peer) = listener.accept().await?;
            let svc = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = svc.handle_tcp_or_websocket(stream, peer).await {
                    tracing::warn!(%peer, error = %e, "tcp/ws connection closed with error");
                }
            });
        }
    }

    async fn handle_tcp_or_websocket(
        self: Arc<Self>,
        stream: TcpStream,
        peer: SocketAddr,
    ) -> Result<()> {
        let ws_path = self.cfg.transport.ws_path.as_str();
        let mut peek_buf = vec![0u8; 4 + ws_path.len()];
        let n = stream.peek(&mut peek_buf).await.unwrap_or(0);
        let physical = if transport::is_websocket_http_request(&peek_buf[..n], ws_path) {
            tracing::debug!(%peer, transport = "websocket", "upgrade");
            transport::accept_websocket(stream).await?
        } else {
            tracing::debug!(%peer, transport = "tcp", "incoming connection");
            boxed_stream(stream)
        };
        let physical = transport::check_and_enable_tls(
            physical,
            Arc::clone(&self.tls_config),
            self.cfg.transport.tls.force,
        )
        .await?;
        self.handle_physical(physical, peer).await
    }

    async fn handle_physical(self: Arc<Self>, physical: DynStream, peer: SocketAddr) -> Result<()> {
        if self.cfg.transport.tcp_mux {
            tracing::debug!(%peer, "yamux server session started");
            let svc = Arc::clone(&self);
            transport::serve_yamux_session(physical, move |stream| {
                let svc = Arc::clone(&svc);
                tokio::spawn(async move {
                    if let Err(e) = svc.handle_connection(stream, peer).await {
                        tracing::debug!(error = %e, "yamux stream closed with error");
                    }
                });
            })
            .await
            .map_err(|e| {
                let msg = e.to_string();

                if msg.contains("unknown version: 65") || msg.contains("unknown version: 87") {
                    anyhow!(
                        "yamux session {peer}: {e} — transport.tcpMux mismatch: server expects yamux \
                         (tcpMux=true) but peer sent a raw control frame (Login 'A'=65 / NewDataConn 'W'=87). \
                         Set the same tcpMux on orbien and orbien-server, then restart both."
                    )
                } else {
                    anyhow!("yamux session {peer}: {e}")
                }
            })
        } else {
            self.handle_connection(physical, peer).await
        }
    }

    pub(super) async fn run_kcp(
        self: Arc<Self>,
        mut listener: kcp_tokio::KcpListener,
    ) -> Result<()> {
        loop {
            let (stream, peer) = transport::accept_kcp(&mut listener).await?;
            tracing::debug!(%peer, transport = "kcp", "incoming connection");
            let svc = Arc::clone(&self);
            tokio::spawn(async move {
                let result = async {
                    let stream = transport::check_and_enable_tls(
                        stream,
                        Arc::clone(&svc.tls_config),
                        svc.cfg.transport.tls.force,
                    )
                    .await?;
                    svc.handle_physical(stream, peer).await
                }
                .await;
                if let Err(e) = result {
                    tracing::warn!(%peer, error = %e, "kcp connection closed with error");
                }
            });
        }
    }

    pub(super) async fn run_quic(self: Arc<Self>, endpoint: quinn::Endpoint) -> Result<()> {
        loop {
            let incoming = endpoint
                .accept()
                .await
                .ok_or_else(|| anyhow!("quic endpoint closed"))?;
            let svc = Arc::clone(&self);
            tokio::spawn(async move {
                match incoming.await {
                    Ok(conn) => {
                        let peer = conn.remote_address();
                        tracing::info!(%peer, "quic session accepted");
                        if let Err(e) = svc.handle_quic_connection(conn).await {
                            tracing::debug!(%peer, error = %e, "quic session ended");
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "quic accept failed"),
                }
            });
        }
    }

    async fn handle_quic_connection(self: Arc<Self>, conn: quinn::Connection) -> Result<()> {
        loop {
            let (send, recv) = conn.accept_bi().await?;
            let stream = transport::quic_bi(send, recv);
            let svc = Arc::clone(&self);
            let peer = conn.remote_address();
            tokio::spawn(async move {
                if let Err(e) = svc.handle_connection(stream, peer).await {
                    tracing::debug!(%peer, error = %e, "quic stream closed with error");
                }
            });
        }
    }

    async fn handle_connection(
        self: Arc<Self>,
        mut stream: DynStream,
        peer: SocketAddr,
    ) -> Result<()> {
        let first = msg::read_msg(&mut stream).await?;
        match first {
            Message::Login(login) => self.register_control(stream, login, peer).await,
            Message::NewDataConn(nw) => self.register_data_conn(stream, nw).await,
            other => Err(anyhow!("unexpected first message: {:?}", other.type_byte())),
        }
    }
}
