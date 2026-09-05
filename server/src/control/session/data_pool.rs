use super::Control;
use anyhow::{anyhow, Result};
use orbien_core::msg::{self, Message, ReqDataConn, StartDataConn};
use orbien_core::transport::DynStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;

impl Control {
    pub async fn push_data_conn(&self, stream: DynStream) {
        let _ = self.data_tx.send(stream).await;
        self.data_notify.notify_waiters();
    }

    async fn try_pop_data(&self) -> Option<DynStream> {
        let mut rx = self.data_rx.lock().await;
        rx.try_recv().ok()
    }

    async fn spawn_refill(self: &Arc<Self>) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        let ctl = Arc::clone(self);
        self.spawn_bg(async move {
            if ctl.closed.load(Ordering::SeqCst) {
                return;
            }
            let _ = ctl.request_data_conn().await;
        })
        .await;
    }

    pub async fn get_data_conn(self: &Arc<Self>) -> Result<DynStream> {
        if let Some(conn) = self.try_pop_data().await {
            self.spawn_refill().await;
            return Ok(conn);
        }

        self.request_data_conn().await?;

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.closed.load(Ordering::SeqCst) {
                return Err(anyhow!("control closed while waiting for data conn"));
            }
            if let Some(conn) = self.try_pop_data().await {
                self.spawn_refill().await;
                return Ok(conn);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("timeout waiting for data conn"));
            }
            tokio::select! {
                _ = self.data_notify.notified() => {}
                _ = sleep(remaining.min(Duration::from_millis(100))) => {}
            }
        }
    }

    pub(super) async fn request_data_conn(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(anyhow!("control closed"));
        }
        let mut writer = self.writer.lock().await;
        msg::write_msg(&mut *writer, &Message::ReqDataConn(ReqDataConn {})).await?;
        Ok(())
    }

    pub async fn start_data_conn(
        &self,
        mut data: DynStream,
        tunnel_name: &str,
        src_addr: String,
        src_port: u16,
        dst_addr: String,
        dst_port: u16,
    ) -> Result<DynStream> {
        msg::write_msg(
            &mut data,
            &Message::StartDataConn(StartDataConn {
                tunnel_name: tunnel_name.to_string(),
                src_addr,
                src_port,
                dst_addr,
                dst_port,
                error: String::new(),
            }),
        )
        .await?;
        Ok(data)
    }
}
