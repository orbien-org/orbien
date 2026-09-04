use super::stream::{boxed_stream, DynStream};
use anyhow::{Context, Result};
use bytes::BytesMut;
use futures_util::{Sink, Stream};
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{accept_async, client_async, WebSocketStream};

pub const ORBIEN_WEBSOCKET_PATH: &str = "/~!orbien";

pub fn normalize_ws_path(path: &str) -> String {
    let t = path.trim();
    if t.is_empty() {
        ORBIEN_WEBSOCKET_PATH.to_string()
    } else {
        t.to_string()
    }
}

pub fn is_websocket_http_request(peeked: &[u8], path: &str) -> bool {
    let prefix = format!("GET {path}");
    peeked.starts_with(prefix.as_bytes())
}

pub async fn accept_websocket(stream: TcpStream) -> Result<DynStream> {
    let ws = accept_async(stream)
        .await
        .context("websocket server accept/upgrade")?;
    Ok(WsByteStream::new(ws).boxed())
}

pub async fn dial_websocket(endpoint: &str, path: &str) -> Result<DynStream> {
    let stream = TcpStream::connect(endpoint)
        .await
        .with_context(|| format!("tcp dial for websocket {endpoint}"))?;
    crate::net::enable_nodelay(&stream);
    let url = format!("ws://{endpoint}{path}");
    let (ws, _resp) = client_async(&url, stream)
        .await
        .with_context(|| format!("websocket client upgrade {url}"))?;
    Ok(WsByteStream::new(ws).boxed())
}

pub struct WsByteStream<S> {
    inner: WebSocketStream<S>,
    read_buf: BytesMut,
}

impl<S> WsByteStream<S> {
    pub fn new(inner: WebSocketStream<S>) -> Self {
        Self {
            inner,
            read_buf: BytesMut::new(),
        }
    }

    pub fn boxed(self) -> DynStream
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        boxed_stream(self)
    }
}

impl<S> AsyncRead for WsByteStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();

        loop {
            if !this.read_buf.is_empty() {
                let n = buf.remaining().min(this.read_buf.len());
                buf.put_slice(&this.read_buf[..n]);
                let _ = this.read_buf.split_to(n);
                return Poll::Ready(Ok(()));
            }

            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(Message::Binary(data)))) => {
                    this.read_buf.extend_from_slice(&data);
                }
                Poll::Ready(Some(Ok(Message::Text(text)))) => {
                    this.read_buf.extend_from_slice(text.as_bytes());
                }
                Poll::Ready(Some(Ok(Message::Ping(_))))
                | Poll::Ready(Some(Ok(Message::Pong(_)))) => {
                    continue;
                }
                Poll::Ready(Some(Ok(Message::Close(_)))) | Poll::Ready(None) => {
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Some(Ok(Message::Frame(_)))) => continue,
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(std::io::Error::other(e)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S> AsyncWrite for WsByteStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        let mut sink = Pin::new(&mut this.inner);

        match sink.as_mut().poll_ready(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(std::io::Error::other(e))),
            Poll::Pending => return Poll::Pending,
        }

        if let Err(e) = sink
            .as_mut()
            .start_send(Message::Binary(bytes::Bytes::copy_from_slice(buf)))
        {
            return Poll::Ready(Err(std::io::Error::other(e)));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match Pin::new(&mut self.get_mut().inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::other(e))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match Pin::new(&mut self.get_mut().inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::other(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}
