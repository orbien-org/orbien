pub(crate) mod model;
mod routes;
mod snapshot;

use crate::service::Service;
use anyhow::Result;
use axum::middleware;
use axum::serve::Listener;
use orbien_core::config::DashboardConfig;
use orbien_core::transport::load_server_cert_key;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

pub async fn run(svc: Arc<Service>, cfg: DashboardConfig) -> Result<()> {
    let addr = format!("{}:{}", cfg.addr, cfg.port);
    let listener = TcpListener::bind(&addr).await?;

    let state = Arc::new(DashState {
        svc,
        cfg: cfg.clone(),
    });
    let app = routes::router(state.clone())
        .layer(middleware::from_fn_with_state(state, routes::basic_auth))
        .into_make_service();

    if cfg.tls.enabled() {
        let tls_cfg = load_server_cert_key(&cfg.tls.cert_file, &cfg.tls.key_file)?;
        let acceptor = TlsAcceptor::from(tls_cfg);
        tracing::info!(%addr, user = %cfg.user, "dashboard listening (https)");
        axum::serve(TlsListener { listener, acceptor }, app).await?;
    } else {
        tracing::info!(%addr, user = %cfg.user, "dashboard listening");
        axum::serve(listener, app).await?;
    }
    Ok(())
}

struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (tcp, peer) = match self.listener.accept().await {
                Ok(tup) => tup,
                Err(e) => {
                    tracing::debug!(error = %e, "dashboard accept failed");
                    continue;
                }
            };
            match self.acceptor.accept(tcp).await {
                Ok(tls) => return (tls, peer),
                Err(e) => {
                    tracing::debug!(%peer, error = %e, "dashboard TLS handshake failed");
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

#[derive(Clone)]
pub struct DashState {
    pub svc: Arc<Service>,
    pub cfg: DashboardConfig,
}
