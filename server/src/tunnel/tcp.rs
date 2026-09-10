use crate::access::prepare_ingress;
use crate::control::Control;
use crate::metrics;
use anyhow::Result;
use orbien_core::limit::{maybe_limit, BandwidthLimiter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;

pub struct TcpTunnel {
    pub name: String,
    pub remote_port: u16,
    closed: Arc<AtomicBool>,
    notify: Arc<Notify>,
    accept_task: Mutex<Option<JoinHandle<()>>>,
}

impl TcpTunnel {
    pub async fn start(
        name: String,
        bind_addr: String,
        remote_port: u16,
        control: Arc<Control>,
        limiter: Option<Arc<BandwidthLimiter>>,
    ) -> Result<Self> {
        let addr = format!("{bind_addr}:{remote_port}");
        let listener = TcpListener::bind(&addr).await?;
        tracing::info!(%addr, tunnel = %name, "tcp tunnel listening");

        let closed = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let closed_flag = Arc::clone(&closed);
        let notify_wait = Arc::clone(&notify);
        let tunnel_name = name.clone();
        let limiter_spawn = limiter.clone();

        let control_weak = Arc::downgrade(&control);

        let accept_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = notify_wait.notified() => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, peer)) => {
                                orbien_core::net::enable_nodelay(&stream);
                                if closed_flag.load(Ordering::SeqCst) {
                                    break;
                                }
                                let Some(ctl) = control_weak.upgrade() else {
                                    break;
                                };
                                let pname = tunnel_name.clone();
                                let lim = limiter_spawn.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = handle_ingress(ctl, &pname, stream, peer, lim).await
                                    {
                                        tracing::debug!(tunnel = %pname, error = %e, "ingress ended");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "accept failed");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            name,
            remote_port,
            closed,
            notify,
            accept_task: Mutex::new(Some(accept_task)),
        })
    }

    pub async fn close(&self) {
        tracing::info!(tunnel = %self.name, remote_port = self.remote_port, "tcp tunnel closing");
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        if let Some(h) = self.accept_task.lock().await.take() {
            h.abort();
            let _ = h.await;
        }
    }
}

impl Drop for TcpTunnel {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        if let Some(h) = self.accept_task.get_mut().take() {
            h.abort();
        }
    }
}

async fn handle_ingress(
    control: Arc<Control>,
    tunnel_name: &str,
    stream: tokio::net::TcpStream,
    peer: std::net::SocketAddr,
    limiter: Option<Arc<BandwidthLimiter>>,
) -> Result<()> {
    let ingress = prepare_ingress(stream, peer);
    let data = control.get_data_conn().await?;
    let data = control
        .start_data_conn(
            data,
            tunnel_name,
            ingress.source.ip().to_string(),
            ingress.source.port(),
            ingress
                .local
                .map(|a| a.ip().to_string())
                .unwrap_or_default(),
            ingress.local.map(|a| a.port()).unwrap_or(0),
        )
        .await?;

    let data = maybe_limit(data, limiter);

    tracing::debug!(
        tunnel = %tunnel_name,
        peer = %ingress.peer,
        source = %ingress.source,
        "joining ingress <-> data"
    );
    let _ =
        metrics::join_and_record(&control.metrics, tunnel_name, "tcp", ingress.stream, data).await;
    Ok(())
}
