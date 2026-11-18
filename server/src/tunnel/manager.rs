use super::{HttpTunnel, HttpsTunnel, TcpTunnel, UdpTunnel};
use std::collections::HashMap;

pub enum RegisteredTunnel {
    Tcp(TcpTunnel),
    Http(HttpTunnel),
    Https(HttpsTunnel),
    Udp(UdpTunnel),
}

impl RegisteredTunnel {
    pub fn tunnel_type(&self) -> &'static str {
        match self {
            Self::Tcp(_) => "tcp",
            Self::Http(_) => "http",
            Self::Https(_) => "https",
            Self::Udp(_) => "udp",
        }
    }

    pub fn remote_port(&self) -> Option<u16> {
        match self {
            Self::Tcp(t) => Some(t.remote_port),
            Self::Udp(u) => Some(u.remote_port),
            Self::Http(_) | Self::Https(_) => None,
        }
    }

    pub async fn close(&self) {
        match self {
            Self::Tcp(p) => p.close().await,
            Self::Http(p) => p.close().await,
            Self::Https(p) => p.close().await,
            Self::Udp(p) => p.close().await,
        }
    }
}

pub struct DetachedTunnel {
    pub tunnel_type: &'static str,
    pub remote_port: Option<u16>,
}

struct TunnelEntry {
    tunnel: RegisteredTunnel,
    local_addr: String,
}

pub struct TunnelManager {
    tunnels: HashMap<String, TunnelEntry>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            tunnels: HashMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        name: String,
        tunnel: RegisteredTunnel,
        local_addr: String,
    ) -> Result<(), RegisteredTunnel> {
        use std::collections::hash_map::Entry;
        match self.tunnels.entry(name) {
            Entry::Vacant(slot) => {
                slot.insert(TunnelEntry { tunnel, local_addr });
                Ok(())
            }
            Entry::Occupied(_) => Err(tunnel),
        }
    }

    pub async fn remove(&mut self, name: &str) -> Option<DetachedTunnel> {
        if let Some(entry) = self.tunnels.remove(name) {
            let detached = DetachedTunnel {
                tunnel_type: entry.tunnel.tunnel_type(),
                remote_port: entry.tunnel.remote_port(),
            };
            entry.tunnel.close().await;
            Some(detached)
        } else {
            None
        }
    }

    /// Remove every tunnel from the manager without awaiting any close.
    /// Callers can release global slots and close the returned tunnels after
    /// dropping the manager lock.
    pub fn drain_all(&mut self) -> Vec<(String, RegisteredTunnel)> {
        self.tunnels
            .drain()
            .map(|(name, entry)| (name, entry.tunnel))
            .collect()
    }

    pub fn abandon_all(&mut self) -> Vec<(String, DetachedTunnel)> {
        self.tunnels
            .drain()
            .map(|(name, entry)| {
                let detached = DetachedTunnel {
                    tunnel_type: entry.tunnel.tunnel_type(),
                    remote_port: entry.tunnel.remote_port(),
                };
                (name, detached)
            })
            .collect()
    }

    pub fn summaries(&self) -> Vec<TunnelSummary> {
        self.tunnels
            .iter()
            .map(|(name, entry)| {
                let (tunnel_type, remote_addr) = match &entry.tunnel {
                    RegisteredTunnel::Tcp(t) => ("tcp".into(), format!(":{}", t.remote_port)),
                    RegisteredTunnel::Http(h) => ("http".into(), h.domains.join(",")),
                    RegisteredTunnel::Https(h) => ("https".into(), h.domains.join(",")),
                    RegisteredTunnel::Udp(u) => ("udp".into(), format!(":{}", u.remote_port)),
                };
                TunnelSummary {
                    name: name.clone(),
                    tunnel_type,
                    remote_addr,
                    local_addr: entry.local_addr.clone(),
                    status: "online".into(),
                }
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.tunnels.len()
    }
}

pub fn format_local_addr(ip: &str, port: i32) -> String {
    let ip = ip.trim();
    if ip.is_empty() && port <= 0 {
        return String::new();
    }
    if ip.is_empty() {
        return format!(":{port}");
    }
    if port <= 0 {
        return ip.to_string();
    }
    if ip.contains(':') && !ip.starts_with('[') {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TunnelSummary {
    pub name: String,
    #[serde(rename = "type")]
    pub tunnel_type: String,
    #[serde(rename = "remoteAddr")]
    pub remote_addr: String,
    #[serde(rename = "localAddr")]
    pub local_addr: String,
    pub status: String,
}
