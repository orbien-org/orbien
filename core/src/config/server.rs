use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,

    #[serde(default, rename = "quicPort", alias = "quic_port")]
    pub quic_port: u16,

    #[serde(default, rename = "kcpPort", alias = "kcp_port")]
    pub kcp_port: u16,

    #[serde(default, rename = "httpGwPort", alias = "http_gw_port")]
    pub http_gw_port: u16,

    #[serde(default, rename = "httpsGwPort", alias = "https_gw_port")]
    pub https_gw_port: u16,

    #[serde(default, rename = "rootDomain", alias = "root_domain")]
    pub root_domain: String,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(
        default = "default_proxy_addr",
        rename = "proxyAddr",
        alias = "proxy_addr"
    )]
    pub proxy_addr: String,

    #[serde(default)]
    pub transport: ServerTransportConfig,

    #[serde(default)]
    pub dashboard: DashboardConfig,

    #[serde(
        default = "default_udp_packet_size",
        rename = "udpPacketSize",
        alias = "udp_packet_size"
    )]
    pub udp_packet_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default = "default_auth_type", rename = "type", alias = "auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTransportConfig {
    #[serde(default = "default_tcp_mux", rename = "tcpMux", alias = "tcp_mux")]
    pub tcp_mux: bool,

    #[serde(
        default = "default_tcp_mux_keepalive",
        rename = "muxKeepaliveSecs",
        alias = "mux_keepalive_secs"
    )]
    pub mux_keepalive_secs: i64,

    #[serde(default, rename = "maxConnPool", alias = "max_conn_pool")]
    pub max_conn_pool: i64,

    #[serde(default, rename = "heartbeatTimeout", alias = "heartbeat_timeout")]
    pub heartbeat_timeout: i64,
    #[serde(default)]
    pub quic: QuicOptions,

    #[serde(default = "default_ws_path", rename = "wsPath", alias = "ws_path")]
    pub ws_path: String,

    #[serde(default)]
    pub tls: ServerTlsConfig,
}

impl Default for ServerTransportConfig {
    fn default() -> Self {
        Self {
            tcp_mux: default_tcp_mux(),
            mux_keepalive_secs: default_tcp_mux_keepalive(),
            max_conn_pool: 0,
            heartbeat_timeout: 0,
            quic: QuicOptions::default(),
            ws_path: default_ws_path(),
            tls: ServerTlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardConfig {
    #[serde(default)]
    pub addr: String,

    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

impl DashboardConfig {
    pub fn complete(&mut self) {
        if self.addr.trim().is_empty() {
            self.addr = "127.0.0.1".into();
        }
    }

    pub fn enabled(&self) -> bool {
        self.port > 0
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled() {
            return Ok(());
        }
        let user = self.user.trim();
        let pass = self.password.trim();
        if user.is_empty() || pass.is_empty() {
            anyhow::bail!(
                "dashboard.user and dashboard.password are required when dashboard.port > 0"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerTlsConfig {
    #[serde(default)]
    pub force: bool,
    #[serde(default, rename = "certFile", alias = "cert_file")]
    pub cert_file: String,
    #[serde(default, rename = "keyFile", alias = "key_file")]
    pub key_file: String,
    #[serde(default, rename = "trustedCaFile", alias = "trusted_ca_file")]
    pub trusted_ca_file: String,
}

fn default_tcp_mux() -> bool {
    true
}

fn default_ws_path() -> String {
    crate::transport::ORBIEN_WEBSOCKET_PATH.to_string()
}

fn default_tcp_mux_keepalive() -> i64 {
    30
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicOptions {
    #[serde(
        default = "default_quic_keepalive",
        rename = "keepalivePeriod",
        alias = "keepalive_period"
    )]
    pub keepalive_period: u64,
    #[serde(
        default = "default_quic_idle",
        rename = "maxIdleTimeout",
        alias = "max_idle_timeout"
    )]
    pub max_idle_timeout: u64,
    #[serde(
        default = "default_quic_streams",
        rename = "maxIncomingStreams",
        alias = "max_incoming_streams"
    )]
    pub max_incoming_streams: u32,
}

impl Default for QuicOptions {
    fn default() -> Self {
        Self {
            keepalive_period: default_quic_keepalive(),
            max_idle_timeout: default_quic_idle(),
            max_incoming_streams: default_quic_streams(),
        }
    }
}

impl QuicOptions {
    pub fn keepalive(&self) -> Duration {
        Duration::from_secs(self.keepalive_period.max(1))
    }

    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.max_idle_timeout.max(1))
    }
}

fn default_listen() -> String {
    format!("{}:{}", default_listen_host(), default_listen_port())
}

fn default_listen_host() -> String {
    "0.0.0.0".into()
}

fn default_listen_port() -> u16 {
    9527
}

fn default_proxy_addr() -> String {
    default_listen_host()
}

fn default_auth_type() -> String {
    "token".into()
}

fn default_quic_keepalive() -> u64 {
    10
}

fn default_quic_idle() -> u64 {
    30
}

fn default_quic_streams() -> u32 {
    100_000
}

fn default_udp_packet_size() -> usize {
    1500
}

pub fn parse_host_port(raw: &str, default_port: u16) -> anyhow::Result<(String, u16)> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok((default_listen_host(), default_port));
    }

    if let Some(rest) = s.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| anyhow!("invalid listen address '{raw}': missing ']'"))?;
        if host.is_empty() {
            return Err(anyhow!("invalid listen address '{raw}': empty IPv6 host"));
        }
        let host = format!("[{host}]");
        if after.is_empty() {
            return Ok((host, default_port));
        }
        let port_str = after
            .strip_prefix(':')
            .ok_or_else(|| anyhow!("invalid listen address '{raw}': expected ':' after ']'"))?;
        if port_str.is_empty() {
            return Ok((host, default_port));
        }
        let port: u16 = port_str
            .parse()
            .with_context(|| format!("invalid listen port in '{raw}'"))?;
        if port == 0 {
            return Ok((host, default_port));
        }
        return Ok((host, port));
    }

    if let Some((host, port_str)) = s.rsplit_once(':') {
        if !host.is_empty() && !host.contains(':') {
            if port_str.is_empty() {
                return Ok((host.to_string(), default_port));
            }
            let port: u16 = port_str
                .parse()
                .with_context(|| format!("invalid listen port in '{raw}'"))?;
            if port == 0 {
                return Ok((host.to_string(), default_port));
            }
            return Ok((host.to_string(), port));
        }
    }

    Ok((s.to_string(), default_port))
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            quic_port: 0,
            kcp_port: 0,
            http_gw_port: 0,
            https_gw_port: 0,
            root_domain: String::new(),
            auth: AuthConfig::default(),
            proxy_addr: default_proxy_addr(),
            transport: ServerTransportConfig::default(),
            dashboard: DashboardConfig::default(),
            udp_packet_size: default_udp_packet_size(),
        }
    }
}

impl ServerConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let file = super::read_toml_file(path)?;
        let expanded = super::expand_env_placeholders(&file)?;
        let mut cfg: Self = toml::from_str(&expanded)
            .with_context(|| format!("failed to parse config file '{}'", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        cfg.resolve_paths(base);
        cfg.complete();
        cfg.validate()?;
        Ok(cfg)
    }

    fn resolve_paths(&mut self, base: &Path) {
        let tls = &mut self.transport.tls;
        tls.cert_file = super::resolve_maybe_relative(base, &tls.cert_file);
        tls.key_file = super::resolve_maybe_relative(base, &tls.key_file);
        tls.trusted_ca_file = super::resolve_maybe_relative(base, &tls.trusted_ca_file);
    }

    pub fn from_defaults() -> Self {
        let mut cfg = Self::default();
        cfg.complete();
        cfg
    }

    pub fn listen_host(&self) -> anyhow::Result<String> {
        Ok(parse_host_port(&self.listen, default_listen_port())?.0)
    }

    pub fn listen_port(&self) -> anyhow::Result<u16> {
        Ok(parse_host_port(&self.listen, default_listen_port())?.1)
    }

    pub fn complete(&mut self) {
        match parse_host_port(&self.listen, default_listen_port()) {
            Ok((host, port)) => {
                self.listen = format!("{host}:{port}");
            }
            Err(_) => {
                self.listen = default_listen();
            }
        }
        if self.proxy_addr.trim().is_empty() {
            self.proxy_addr = self.listen_host().unwrap_or_else(|_| default_listen_host());
        }
        if self.auth.auth_type.trim().is_empty() {
            self.auth.auth_type = default_auth_type();
        }
        if self.udp_packet_size == 0 {
            self.udp_packet_size = default_udp_packet_size();
        }

        self.dashboard.complete();
        if self.transport.max_conn_pool == 0 {
            self.transport.max_conn_pool = 5;
        }
        if self.transport.heartbeat_timeout == 0 {
            self.transport.heartbeat_timeout = if self.transport.tcp_mux { -1 } else { 90 };
        }
        if !self.transport.tls.trusted_ca_file.trim().is_empty() {
            self.transport.tls.force = true;
        }

        self.transport.ws_path = crate::transport::normalize_ws_path(&self.transport.ws_path);
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let (listen_host, listen_port) = parse_host_port(&self.listen, default_listen_port())
            .with_context(|| format!("invalid listen '{}'", self.listen))?;

        if self.quic_enabled() && self.kcp_enabled() && self.quic_port == self.kcp_port {
            anyhow::bail!(
                "quicPort and kcpPort both use UDP and must differ (got {})",
                self.quic_port
            );
        }

        if self.http_gw_enabled()
            && tcp_listen_conflicts(
                &listen_host,
                listen_port,
                &self.proxy_addr,
                self.http_gw_port,
            )
        {
            anyhow::bail!(
                "httpGwPort ({}) must not share a TCP listen with listen ({}) \
                 on overlapping addresses (listen host={}, proxyAddr={}). \
                 HTTP gateway and the control/WebSocket listener are separate sockets; \
                 put HTTP on 80 (or another free port), keep control on listen.",
                self.http_gw_port,
                self.listen,
                listen_host,
                self.proxy_addr
            );
        }

        if self.https_gw_enabled()
            && tcp_listen_conflicts(
                &listen_host,
                listen_port,
                &self.proxy_addr,
                self.https_gw_port,
            )
        {
            anyhow::bail!(
                "httpsGwPort ({}) must not share a TCP listen with listen ({}) \
                 on overlapping addresses (listen host={}, proxyAddr={}). \
                 HTTPS visitors and control TLS both start with 0x16; Orbien does not \
                 mux them on one port. Use 443 (or another free port) for HTTPS gateway.",
                self.https_gw_port,
                self.listen,
                listen_host,
                self.proxy_addr
            );
        }

        if self.http_gw_enabled()
            && self.https_gw_enabled()
            && self.http_gw_port == self.https_gw_port
        {
            anyhow::bail!(
                "httpGwPort and httpsGwPort must differ (both set to {})",
                self.http_gw_port
            );
        }

        self.dashboard.validate()?;

        Ok(())
    }

    pub fn quic_enabled(&self) -> bool {
        self.quic_port != 0
    }
    pub fn kcp_enabled(&self) -> bool {
        self.kcp_port != 0
    }
    pub fn http_gw_enabled(&self) -> bool {
        self.http_gw_port != 0
    }
    pub fn https_gw_enabled(&self) -> bool {
        self.https_gw_port != 0
    }
}

fn tcp_listen_conflicts(addr_a: &str, port_a: u16, addr_b: &str, port_b: u16) -> bool {
    if port_a == 0 || port_b == 0 || port_a != port_b {
        return false;
    }
    listen_addrs_overlap(addr_a, addr_b)
}

fn listen_addrs_overlap(a: &str, b: &str) -> bool {
    let a = a.trim();
    let b = b.trim();
    if a.is_empty() || b.is_empty() {
        return true;
    }
    if is_unspecified_bind(a) || is_unspecified_bind(b) {
        return true;
    }
    a.eq_ignore_ascii_case(b)
}

fn is_unspecified_bind(addr: &str) -> bool {
    matches!(addr, "0.0.0.0" | "::" | "[::]")
}
