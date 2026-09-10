use super::stream::{boxed_stream, DynStream};
use anyhow::{anyhow, Result};
use std::future::poll_fn;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

const MAX_NUM_STREAMS: usize = 4096;

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
}

impl YamuxClient {
    pub fn start(io: DynStream) -> Self {
        let (open_tx, open_rx) = mpsc::channel::<OpenReply>(64);
        tokio::spawn(drive_client(io, open_rx));
        Self { open_tx }
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
}

async fn drive_client(io: DynStream, mut open_rx: mpsc::Receiver<OpenReply>) {
    let mut conn = yamux::Connection::new(io.compat(), yamux_config(), yamux::Mode::Client);
    loop {
        tokio::select! {
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
                        let _ = poll_fn(|cx| conn.poll_close(cx)).await;
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
