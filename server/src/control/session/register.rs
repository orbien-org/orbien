use super::Control;
use crate::tunnel::{
    format_local_addr, HttpTunnel, HttpsTunnel, PortTable, RegisteredTunnel, TcpTunnel,
    TunnelOwner, UdpTunnel,
};
use anyhow::{anyhow, Result};
use orbien_core::limit::BandwidthLimiter;
use orbien_core::msg::{self, CloseTunnel, Message, NewTunnel, NewTunnelResp};
use std::sync::Arc;

impl Control {
    fn note_tunnel_registered(&self, name: &str, tunnel_type: &str) {
        self.metrics
            .new_tunnel(name, tunnel_type, &self.user, &self.session_id);
    }

    fn tunnel_transport(np: &NewTunnel) -> Result<Option<Arc<BandwidthLimiter>>> {
        let limiter = orbien_core::limit::limiter_if_side(
            np.bandwidth,
            &np.bandwidth_limit_side,
            orbien_core::limit::BandwidthLimitSide::Server,
        )?;
        if let Some(ref l) = limiter {
            tracing::info!(
                tunnel = %np.tunnel_name,
                bytes_per_sec = l.bytes_per_sec(),
                mode = "server",
                "bandwidth limit enabled"
            );
        }
        Ok(limiter)
    }

    pub(super) async fn handle_new_tunnel(self: &Arc<Self>, np: NewTunnel) -> Result<()> {
        let resp = match self.register_tunnel(&np).await {
            Ok(remote_addr) => NewTunnelResp {
                tunnel_name: np.tunnel_name.clone(),
                remote_addr,
                error: String::new(),
            },
            Err(e) => NewTunnelResp {
                tunnel_name: np.tunnel_name.clone(),
                remote_addr: String::new(),
                error: e.to_string(),
            },
        };

        let mut writer = self.writer.lock().await;
        msg::write_msg(&mut *writer, &Message::NewTunnelResp(resp)).await?;
        Ok(())
    }

    async fn register_tunnel(self: &Arc<Self>, np: &NewTunnel) -> Result<String> {
        if self.is_closed() {
            return Err(anyhow!("control session is closed"));
        }
        match np.protocol.as_str() {
            "tcp" => self.register_tcp_tunnel(np).await,
            "http" => self.register_http_tunnel(np).await,
            "https" => self.register_https_tunnel(np).await,
            "udp" => self.register_udp_tunnel(np).await,
            other => Err(anyhow!("unsupported tunnel protocol: {other}")),
        }
    }

    async fn prepare_name_slot(&self, name: &str) {
        if let Some(old_ty) = self.detach_tunnel(name).await {
            self.metrics.close_tunnel(name, old_ty);
        }
    }

    async fn claim_name(&self, name: &str) -> Result<TunnelOwner> {
        let owner = self.owner();
        self.prepare_name_slot(name).await;
        self.tunnel_registry.try_insert(name, owner.clone())?;
        Ok(owner)
    }

    fn release_name(&self, name: &str, owner: &TunnelOwner) {
        self.tunnel_registry.remove_if_owner(name, owner);
    }

    fn claim_port(
        &self,
        ports: &PortTable,
        port: u16,
        name: &str,
        owner: &TunnelOwner,
    ) -> Result<()> {
        if let Err(e) = ports.claim(port, name) {
            self.release_name(name, owner);
            return Err(e);
        }
        Ok(())
    }

    async fn insert_tunnel(
        &self,
        name: String,
        tunnel: RegisteredTunnel,
        local_addr: String,
        remote_addr: String,
        owner: &TunnelOwner,
        ports: Option<(&PortTable, u16)>,
    ) -> Result<String> {
        let ty = tunnel.tunnel_type();
        let mut tm = self.tunnels.lock().await;
        if let Err(tunnel) = tm.insert(name.clone(), tunnel, local_addr) {
            drop(tm);
            tunnel.close().await;
            if let Some((table, port)) = ports {
                table.release(port, &name);
            }
            self.release_name(&name, owner);
            return Err(anyhow!("tunnel `{name}` already present in this session"));
        }
        self.note_tunnel_registered(&name, ty);
        Ok(remote_addr)
    }

    async fn register_tcp_tunnel(self: &Arc<Self>, np: &NewTunnel) -> Result<String> {
        if np.remote_port <= 0 || np.remote_port > 65535 {
            return Err(anyhow!("invalid remote_port"));
        }

        let limiter = Self::tunnel_transport(np)?;
        let bind_addr = self.cfg.proxy_addr.clone();
        let remote_port = np.remote_port as u16;
        let name = np.tunnel_name.clone();
        let owner = self.claim_name(&name).await?;
        self.claim_port(&self.tcp_ports, remote_port, &name, &owner)?;

        let tunnel = match TcpTunnel::start(
            name.clone(),
            bind_addr,
            remote_port,
            Arc::clone(self),
            limiter,
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                self.tcp_ports.release(remote_port, &name);
                self.release_name(&name, &owner);
                return Err(e);
            }
        };

        let remote_addr = format!(":{remote_port}");
        let local_addr = format_local_addr(&np.local_ip, np.local_port);
        let result = self
            .insert_tunnel(
                name,
                RegisteredTunnel::Tcp(tunnel),
                local_addr,
                remote_addr,
                &owner,
                Some((&self.tcp_ports, remote_port)),
            )
            .await?;
        tracing::info!(
            tunnel = %np.tunnel_name,
            port = remote_port,
            session_id = %self.session_id,
            generation = self.generation,
            "tcp tunnel registered"
        );
        Ok(result)
    }

    async fn register_udp_tunnel(self: &Arc<Self>, np: &NewTunnel) -> Result<String> {
        if np.remote_port <= 0 || np.remote_port > 65535 {
            return Err(anyhow!("invalid remote_port"));
        }

        let limiter = Self::tunnel_transport(np)?;
        let bind_addr = self.cfg.proxy_addr.clone();
        let remote_port = np.remote_port as u16;
        let name = np.tunnel_name.clone();
        let packet_size = self.cfg.udp_packet_size.max(512);
        let owner = self.claim_name(&name).await?;
        self.claim_port(&self.udp_ports, remote_port, &name, &owner)?;

        let tunnel = match UdpTunnel::start(
            name.clone(),
            bind_addr,
            remote_port,
            Arc::clone(self),
            limiter,
            packet_size,
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                self.udp_ports.release(remote_port, &name);
                self.release_name(&name, &owner);
                return Err(e);
            }
        };

        let remote_addr = format!(":{remote_port}");
        let local_addr = format_local_addr(&np.local_ip, np.local_port);
        let result = self
            .insert_tunnel(
                name,
                RegisteredTunnel::Udp(tunnel),
                local_addr,
                remote_addr,
                &owner,
                Some((&self.udp_ports, remote_port)),
            )
            .await?;
        tracing::info!(
            tunnel = %np.tunnel_name,
            port = remote_port,
            session_id = %self.session_id,
            generation = self.generation,
            "udp tunnel registered"
        );
        Ok(result)
    }

    async fn register_http_tunnel(self: &Arc<Self>, np: &NewTunnel) -> Result<String> {
        let gw = self
            .http_gw
            .clone()
            .ok_or_else(|| anyhow!("http tunnel requires server httpGwPort > 0"))?;

        let limiter = Self::tunnel_transport(np)?;
        let name = np.tunnel_name.clone();
        let owner = self.claim_name(&name).await?;

        let tunnel = match HttpTunnel::register(
            np,
            Arc::clone(self),
            Arc::clone(&gw),
            &self.cfg.root_domain,
            limiter,
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                self.release_name(&name, &owner);
                return Err(e);
            }
        };

        let remote_addr = tunnel
            .domains
            .iter()
            .map(|d| format!("{d}:{}", gw.listen_port))
            .collect::<Vec<_>>()
            .join(",");
        let local_addr = format_local_addr(&np.local_ip, np.local_port);
        self.insert_tunnel(
            name,
            RegisteredTunnel::Http(tunnel),
            local_addr,
            remote_addr,
            &owner,
            None,
        )
        .await
    }

    async fn register_https_tunnel(self: &Arc<Self>, np: &NewTunnel) -> Result<String> {
        let gw = self
            .https_gw
            .clone()
            .ok_or_else(|| anyhow!("https tunnel requires server httpsGwPort > 0"))?;

        let limiter = Self::tunnel_transport(np)?;
        let name = np.tunnel_name.clone();
        let owner = self.claim_name(&name).await?;

        let tunnel = match HttpsTunnel::register(
            np,
            Arc::clone(self),
            Arc::clone(&gw),
            &self.cfg.root_domain,
            limiter,
        )
        .await
        {
            Ok(t) => t,
            Err(e) => {
                self.release_name(&name, &owner);
                return Err(e);
            }
        };

        let remote_addr = tunnel
            .domains
            .iter()
            .map(|d| format!("{d}:{}", gw.listen_port))
            .collect::<Vec<_>>()
            .join(",");
        let local_addr = format_local_addr(&np.local_ip, np.local_port);
        self.insert_tunnel(
            name,
            RegisteredTunnel::Https(tunnel),
            local_addr,
            remote_addr,
            &owner,
            None,
        )
        .await
    }

    pub(super) async fn handle_close_tunnel(&self, cp: CloseTunnel) -> Result<()> {
        if let Some(ty) = self.detach_tunnel(&cp.tunnel_name).await {
            self.metrics.close_tunnel(&cp.tunnel_name, ty);
        }
        Ok(())
    }
}
