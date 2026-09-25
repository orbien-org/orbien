use super::stream::{boxed_stream, DynStream};
use anyhow::{anyhow, Result};
use std::future::poll_fn;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

const MAX_NUM_STREAMS: usize = 4096;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

fn yamux_config() -> yamux::Config {
    let mut cfg = yamux::Config::default();
    cfg.set_max_num_streams(MAX_NUM_STREAMS);
    cfg.set_max_connection_receive_window(Some(256 * 1024 * MAX_NUM_STREAMS));
    cfg
}

fn box_yamux_stream(stream: yamux::Stream) -> DynStream {
    boxed_stream(stream.compat())
}

type OpenReply = oneshot::Sender<Result<DynStream>>;

pub struct YamuxClient {
    open_tx: mpsc::Sender<OpenReply>,
    stop_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl YamuxClient {
    pub fn start(io: DynStream) -> Self {
        let (open_tx, open_rx) = mpsc::channel::<OpenReply>(64);
        let (stop_tx, stop_rx) = oneshot::channel();
        tokio::spawn(drive_client(io, open_rx, stop_rx));
        Self {
            open_tx,
            stop_tx: Mutex::new(Some(stop_tx)),
        }
    }

    pub async fn open_stream(&self) -> Result<DynStream> {
        let (tx, rx) = oneshot::channel();
        self.open_tx
            .send(tx)
            .await
            .map_err(|_| anyhow!("yamux client session closed"))?;
        rx.await
            .map_err(|_| anyhow!("yamux open_stream cancelled"))?
    }

    pub fn close(&self) {
        if let Some(tx) = self
            .stop_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = tx.send(());
        }
    }
}

async fn drive_client(
    io: DynStream,
    mut open_rx: mpsc::Receiver<OpenReply>,
    mut stop_rx: oneshot::Receiver<()>,
) {
    let mut conn = yamux::Connection::new(io.compat(), yamux_config(), yamux::Mode::Client);
    loop {
        tokio::select! {
            _ = &mut stop_rx => {
                if tokio::time::timeout(CLOSE_TIMEOUT, poll_fn(|cx| conn.poll_close(cx)))
                    .await
                    .is_err()
                {
                    tracing::debug!("yamux client close timed out, dropping connection");
                }
                break;
            }
            cmd = open_rx.recv() => {
                match cmd {
                    Some(reply) => {
                        let res = poll_fn(|cx| conn.poll_new_outbound(cx))
                            .await
                            .map(box_yamux_stream)
                            .map_err(|e| anyhow!("yamux open outbound: {e}"));
                        let _ = reply.send(res);
                    }
                    None => {
                        if tokio::time::timeout(CLOSE_TIMEOUT, poll_fn(|cx| conn.poll_close(cx)))
                            .await
                            .is_err()
                        {
                            tracing::debug!("yamux client close timed out, dropping connection");
                        }
                        break;
                    }
                }
            }
            inbound = poll_fn(|cx| conn.poll_next_inbound(cx)) => {
                match inbound {
                    Some(Ok(_stream)) => {

                        tracing::debug!("yamux client ignored unexpected inbound stream");
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "yamux client session error");
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

pub async fn serve_yamux_session(
    io: DynStream,
    mut on_stream: impl FnMut(DynStream),
) -> Result<()> {
    let mut conn = yamux::Connection::new(io.compat(), yamux_config(), yamux::Mode::Server);
    loop {
        match poll_fn(|cx| conn.poll_next_inbound(cx)).await {
            Some(Ok(stream)) => {
                on_stream(box_yamux_stream(stream));
            }
            Some(Err(e)) => {
                return Err(anyhow!("yamux server accept: {e}"));
            }
            None => return Ok(()),
        }
    }
}
