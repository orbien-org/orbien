use anyhow::{anyhow, Result};
use async_trait::async_trait;
use orbien_core::config::ClientConfig;
use orbien_core::transport::{
    boxed_stream, client_enable_tls, dial_kcp, dial_websocket, new_client_tls_config, DynStream,
    Protocol, QuicSession, YamuxClient,
};
use rustls::ClientConfig as RustlsClientConfig;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use tokio::net::TcpStream;

#[async_trait]
pub trait Connector: Send + Sync {
    async fn open(&self) -> Result<DynStream>;
}

struct TlsDialOpts {
    enable: bool,
    cfg: Arc<RustlsClientConfig>,
    server_name: String,
}

impl TlsDialOpts {
    fn from_config(cfg: &ClientConfig) -> Result<Self> {
        let tls = &cfg.transport.tls;
        let rustls_cfg =
            new_client_tls_config(&tls.cert_file, &tls.key_file, &tls.trusted_ca_file)?;
        Ok(Self {
            enable: tls.enable,
            cfg: rustls_cfg,
            server_name: cfg.tls_server_name().to_string(),
        })
    }

    async fn maybe_wrap(&self, stream: DynStream) -> Result<DynStream> {
        if !self.enable {
            return Ok(stream);
        }
        client_enable_tls(stream, Arc::clone(&self.cfg), &self.server_name).await
    }
}

pub async fn build_connector(cfg: &ClientConfig) -> Result<Arc<dyn Connector>> {
    let tls = TlsDialOpts::from_config(cfg)?;
    match cfg.protocol()? {
        Protocol::Tcp => {
            if cfg.transport.tcp_mux {
                let stream = dial_tcp_tls(cfg, &tls).await?;
                tracing::info!(
                    endpoint = %cfg.server_endpoint(),
                    tls = tls.enable,
                    "tcpMux: physical TCP opened, yamux client started"
                );
                Ok(Arc::new(YamuxConnector {
                    yamux: YamuxClient::start(stream),
                }))
            } else {
                Ok(Arc::new(TcpConnector {
                    endpoint: cfg.server_endpoint(),
                    tls,
                }))
            }
        }
        Protocol::Websocket => {
            if cfg.transport.tcp_mux {
                let stream = dial_ws_tls(cfg, &tls).await?;
                tracing::info!(
                    endpoint = %cfg.server_endpoint(),
                    tls = tls.enable,
                    "tcpMux: physical WebSocket opened, yamux client started"
                );
                Ok(Arc::new(YamuxConnector {
                    yamux: YamuxClient::start(stream),
                }))
            } else {
                Ok(Arc::new(WebsocketConnector {
                    endpoint: cfg.server_endpoint(),
                    ws_path: cfg.transport.ws_path.clone(),
                    tls,
                }))
            }
        }
        Protocol::Kcp => {
            let addr = resolve_addr(cfg)?;
            if cfg.transport.tcp_mux {
                let stream = dial_kcp_tls(addr, &tls).await?;
                tracing::info!(
                    %addr,
                    tls = tls.enable,
                    "tcpMux: physical KCP opened, yamux client started"
                );
                Ok(Arc::new(YamuxConnector {
                    yamux: YamuxClient::start(stream),
                }))
            } else {
                Ok(Arc::new(KcpConnector { addr, tls }))
            }
        }
        Protocol::Quic => {
            let addr = resolve_addr(cfg)?;
            let t = &cfg.transport.tls;
            let session = QuicSession::dial(
                addr,
                &cfg.tls_server_name(),
                cfg.transport.quic.keepalive(),
                cfg.transport.quic.idle_timeout(),
                cfg.transport.quic.max_incoming_streams,
                &t.cert_file,
                &t.key_file,
                &t.trusted_ca_file,
            )
            .await?;
            tracing::info!(%addr, "quic session opened");
            Ok(Arc::new(QuicConnector {
                session: Arc::new(session),
            }))
        }
    }
}

async fn dial_tcp_tls(cfg: &ClientConfig, tls: &TlsDialOpts) -> Result<DynStream> {
    let stream = TcpStream::connect(cfg.server_endpoint()).await?;
    orbien_core::net::enable_nodelay(&stream);
    tls.maybe_wrap(boxed_stream(stream)).await
}

async fn dial_ws_tls(cfg: &ClientConfig, tls: &TlsDialOpts) -> Result<DynStream> {
    let stream = dial_websocket(&cfg.server_endpoint(), &cfg.transport.ws_path).await?;
    tls.maybe_wrap(stream).await
}

async fn dial_kcp_tls(addr: SocketAddr, tls: &TlsDialOpts) -> Result<DynStream> {
    let stream = dial_kcp(addr).await?;
    tls.maybe_wrap(stream).await
}

fn resolve_addr(cfg: &ClientConfig) -> Result<SocketAddr> {
    cfg.server_endpoint()
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("cannot resolve {}", cfg.server_endpoint()))
}

struct YamuxConnector {
    yamux: YamuxClient,
}

#[async_trait]
impl Connector for YamuxConnector {
    async fn open(&self) -> Result<DynStream> {
        self.yamux.open_stream().await
    }
}

struct TcpConnector {
    endpoint: String,
    tls: TlsDialOpts,
}

#[async_trait]
impl Connector for TcpConnector {
    async fn open(&self) -> Result<DynStream> {
        let stream = TcpStream::connect(&self.endpoint).await?;
        orbien_core::net::enable_nodelay(&stream);
        self.tls.maybe_wrap(boxed_stream(stream)).await
    }
}

struct WebsocketConnector {
    endpoint: String,
    ws_path: String,
    tls: TlsDialOpts,
}

#[async_trait]
impl Connector for WebsocketConnector {
    async fn open(&self) -> Result<DynStream> {
        let stream = dial_websocket(&self.endpoint, &self.ws_path).await?;
        self.tls.maybe_wrap(stream).await
    }
}

struct KcpConnector {
    addr: SocketAddr,
    tls: TlsDialOpts,
}

#[async_trait]
impl Connector for KcpConnector {
    async fn open(&self) -> Result<DynStream> {
        let stream = dial_kcp(self.addr).await?;
        self.tls.maybe_wrap(stream).await
    }
}

struct QuicConnector {
    session: Arc<QuicSession>,
}

#[async_trait]
impl Connector for QuicConnector {
    async fn open(&self) -> Result<DynStream> {
        self.session.open_stream().await
    }
}
