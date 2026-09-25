use super::udp::run_udp_session;
use crate::plugin::{self, ConnectionInfo, Plugin, PluginContext};
use anyhow::{anyhow, Result};
use orbien_core::config::{ClientConfig, TunnelConfig};
use orbien_core::io;
use orbien_core::limit::{self, maybe_limit, BandwidthLimitSide, BandwidthLimiter};
use orbien_core::msg::StartDataConn;
use orbien_core::net::{
    addrs_from_start_data_conn, build_proxy_protocol_header, parse_proxy_protocol_version,
    TcpKeepaliveConfig,
};
use orbien_core::transport::DynStream;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex as AsyncMutex};

struct TunnelEntry {
    cfg: TunnelConfig,
    limiter: Option<Arc<BandwidthLimiter>>,
    plugin: Option<Arc<dyn Plugin>>,
    proxy_protocol: Option<&'static str>,
    tcp_keepalive: TcpKeepaliveConfig,
    udp_cancel: AsyncMutex<Option<oneshot::Sender<()>>>,
}

pub struct TunnelManager {
    by_name: RwLock<HashMap<String, Arc<TunnelEntry>>>,
    udp_packet_size: usize,
    tcp_keepalive: TcpKeepaliveConfig,
}

impl TunnelManager {
    pub fn from_config(cfg: &ClientConfig) -> Result<Self> {
        let mgr = Self {
            by_name: RwLock::new(HashMap::new()),
            udp_packet_size: cfg.udp_packet_size.max(512),
            tcp_keepalive: cfg.transport.tcp_keepalive(),
        };
        for t in &cfg.tunnels {
            mgr.upsert(t)?;
        }
        Ok(mgr)
    }

    pub fn upsert(&self, tunnel: &TunnelConfig) -> Result<()> {
        let entry = build_entry(tunnel, self.tcp_keepalive)?;
        self.by_name
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tunnel.name.clone(), entry);
        Ok(())
    }

    pub fn remove(&self, name: &str) {
        if let Some(entry) = self
            .by_name
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name)
        {
            cancel_udp_sync(&entry);
        }
    }

    pub fn tunnel_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .by_name
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }

    pub async fn handle_data_conn(&self, start: &StartDataConn, data: DynStream) -> Result<()> {
        let entry = self
            .by_name
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&start.tunnel_name)
            .cloned()
            .ok_or_else(|| anyhow!("unknown tunnel: {}", start.tunnel_name))?;
        let udp_packet_size = self.udp_packet_size;
        serve_entry(entry, udp_packet_size, start, data).await
    }

    async fn handle_stream(
        entry: &Arc<TunnelEntry>,
        start: &StartDataConn,
        data: DynStream,
    ) -> Result<()> {
        let data = maybe_limit(data, entry.limiter.clone());

        if let Some(ref plugin) = entry.plugin {
            tracing::debug!(
                tunnel = %entry.cfg.name,
                plugin = plugin.name(),
                "handle by plugin"
            );
            return plugin
                .handle(ConnectionInfo {
                    stream: data,
                    src_addr: start.src_addr.clone(),
                    src_port: start.src_port,
                    dst_addr: start.dst_addr.clone(),
                    dst_port: start.dst_port,
                })
                .await;
        }

        let (svc_host, svc_port) = entry.cfg.service_host_port()?;
        let local_addr = entry.cfg.service.trim().to_string();
        if local_addr.is_empty() || svc_port == 0 {
            return Err(anyhow!(
                "tunnel {} has empty or invalid service (local backend)",
                entry.cfg.name
            ));
        }
        let local = TcpStream::connect(&local_addr).await.map_err(|e| {
            anyhow!(
                "dial local {} for tunnel {}: {}",
                local_addr,
                entry.cfg.name,
                e
            )
        })?;
        orbien_core::net::tune_tcp_stream(&local, entry.tcp_keepalive);
        let mut local = local;

        if let Some(ver) = entry.proxy_protocol {
            if let Some((src, dst)) = addrs_from_start_data_conn(
                &start.src_addr,
                start.src_port,
                &start.dst_addr,
                start.dst_port,
                svc_port,
            ) {
                let hdr = build_proxy_protocol_header(src, dst, ver)?;
                local.write_all(&hdr).await?;
                tracing::debug!(
                    tunnel = %entry.cfg.name,
                    version = ver,
                    %src,
                    %dst,
                    "wrote PROXY Protocol header to local"
                );
            } else {
                tracing::debug!(
                    tunnel = %entry.cfg.name,
                    "PROXY Protocol configured but StartDataConn src empty; skip"
                );
            }
        }

        tracing::debug!(
            tunnel = %entry.cfg.name,
            %local_addr,
            host = %svc_host,
            limited = entry.limiter.is_some(),
            "joining data <-> local"
        );
        if let Err(e) = io::join(data, local).await {
            tracing::debug!(error = %e, "join ended");
        }
        Ok(())
    }

    async fn handle_udp(
        entry: Arc<TunnelEntry>,
        udp_packet_size: usize,
        data: DynStream,
    ) -> Result<()> {
        let local_addr: std::net::SocketAddr = entry
            .cfg
            .service
            .trim()
            .parse()
            .map_err(|e| anyhow!("invalid local udp service {}: {e}", entry.cfg.service))?;

        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut slot = entry.udp_cancel.lock().await;
            if let Some(old) = slot.take() {
                let _ = old.send(());
            }
            *slot = Some(cancel_tx);
        }

        tracing::info!(
            tunnel = %entry.cfg.name,
            %local_addr,
            "udp data conn; starting forwarder"
        );

        let data = maybe_limit(data, entry.limiter.clone());

        run_udp_session(
            data,
            local_addr,
            udp_packet_size,
            entry.proxy_protocol.map(|s| s.to_string()),
            cancel_rx,
        )
        .await
    }
}

async fn serve_entry(
    entry: Arc<TunnelEntry>,
    udp_packet_size: usize,
    start: &StartDataConn,
    data: DynStream,
) -> Result<()> {
    match entry.cfg.protocol.as_str() {
        "udp" => TunnelManager::handle_udp(entry, udp_packet_size, data).await,
        "tcp" | "http" | "https" => TunnelManager::handle_stream(&entry, start, data).await,
        other => Err(anyhow!(
            "unsupported tunnel protocol on data conn: {} ({})",
            other,
            entry.cfg.name
        )),
    }
}

fn build_entry(
    tunnel: &TunnelConfig,
    tcp_keepalive: TcpKeepaliveConfig,
) -> Result<Arc<TunnelEntry>> {
    let limiter = limit::limiter_if_side(
        tunnel.transport.bandwidth,
        &tunnel.transport.bandwidth_limit_side,
        BandwidthLimitSide::Client,
    )
    .unwrap_or_else(|e| {
        tracing::warn!(
            tunnel = %tunnel.name,
            error = %e,
            "invalid bandwidth; ignoring"
        );
        None
    });
    if let Some(ref l) = limiter {
        tracing::info!(
            tunnel = %tunnel.name,
            mbps = tunnel.transport.bandwidth,
            bytes_per_sec = l.bytes_per_sec(),
            side = "client",
            "bandwidth limit enabled"
        );
    }

    let plugin = if let Some(ref pc) = tunnel.plugin {
        if pc.plugin_type.is_empty() {
            None
        } else {
            let cn = pick_cert_common_name(&tunnel.domains, &tunnel.name);
            let ctx = PluginContext {
                name: tunnel.name.clone(),
                cert_common_name: cn,
                tcp_keepalive,
            };
            Some(plugin::create(ctx, pc)?)
        }
    } else {
        None
    };

    let proxy_protocol = parse_proxy_protocol_version(&tunnel.transport.proxy_protocol_version)?;
    if let Some(ver) = proxy_protocol {
        tracing::info!(
            tunnel = %tunnel.name,
            version = ver,
            "PROXY Protocol enabled (client writes header to local)"
        );
    }

    Ok(Arc::new(TunnelEntry {
        cfg: tunnel.clone(),
        limiter,
        plugin,
        proxy_protocol,
        tcp_keepalive,
        udp_cancel: AsyncMutex::new(None),
    }))
}

fn cancel_udp_sync(entry: &Arc<TunnelEntry>) {
    if let Ok(mut slot) = entry.udp_cancel.try_lock() {
        if let Some(tx) = slot.take() {
            let _ = tx.send(());
        }
    }
}

fn pick_cert_common_name(domains: &[String], tunnel_name: &str) -> String {
    for d in domains {
        let d = d.trim();
        if d.is_empty() {
            continue;
        }
        if d.contains('.') && !d.contains('*') {
            return d.to_ascii_lowercase();
        }
    }
    for d in domains {
        let d = d.trim();
        if !d.is_empty() {
            return d.to_ascii_lowercase();
        }
    }
    tunnel_name.trim().to_ascii_lowercase()
}
