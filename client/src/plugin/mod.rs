mod socks5;
mod tls_term;

use anyhow::{bail, Result};
use async_trait::async_trait;
use orbien_core::config::PluginConfig;
use orbien_core::net::TcpKeepaliveConfig;
use orbien_core::transport::DynStream;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub name: String,
    pub cert_common_name: String,
    pub tcp_keepalive: TcpKeepaliveConfig,
}

#[allow(dead_code)]
pub struct ConnectionInfo {
    pub stream: DynStream,
    pub src_addr: String,
    pub src_port: u16,
    pub dst_addr: String,
    pub dst_port: u16,
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    async fn handle(&self, conn: ConnectionInfo) -> Result<()>;
}

pub fn create(ctx: PluginContext, cfg: &PluginConfig) -> Result<Arc<dyn Plugin>> {
    match cfg.plugin_type.trim().to_ascii_lowercase().as_str() {
        "tls-term" => {
            let p = tls_term::TlsTermPlugin::new(ctx, cfg)?;
            Ok(Arc::new(p))
        }
        "socks5" => {
            let p = socks5::Socks5Plugin::new(ctx, cfg)?;
            Ok(Arc::new(p))
        }
        other => bail!("unknown client plugin type: {other}"),
    }
}
