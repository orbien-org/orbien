use super::server::{parse_host_port, QuicOptions};
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(default = "default_server")]
    pub server: String,

    #[serde(default)]
    pub user: String,

    #[serde(default, rename = "agentId", alias = "agent_id")]
    pub agent_id: String,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub transport: TransportConfig,

    #[serde(default)]
    pub tunnels: Vec<TunnelConfig>,

    #[serde(
        default = "default_udp_packet_size",
        rename = "udpPacketSize",
        alias = "udp_packet_size"
    )]
    pub udp_packet_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    #[serde(default = "default_auth_type", rename = "type", alias = "auth_type")]
    pub auth_type: String,
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(default = "default_protocol")]
    pub protocol: String,

    #[serde(
        default = "default_pool_count",
        rename = "poolCount",
        alias = "pool_count"
    )]
    pub pool_count: i32,

    #[serde(default = "default_tcp_mux", rename = "tcpMux", alias = "tcp_mux")]
    pub tcp_mux: bool,

    #[serde(
        default = "default_mux_keepalive_secs",
        rename = "muxKeepaliveSecs",
        alias = "mux_keepalive_secs"
    )]
    pub mux_keepalive_secs: i64,

    #[serde(
        default = "default_heartbeat_interval",
        rename = "heartbeatInterval",
        alias = "heartbeat_interval"
    )]
    pub heartbeat_interval: i64,
    #[serde(
        default = "default_heartbeat_timeout",
        rename = "heartbeatTimeout",
        alias = "heartbeat_timeout"
    )]
    pub heartbeat_timeout: i64,
    #[serde(default)]
    pub quic: QuicOptions,

    #[serde(default = "default_ws_path", rename = "wsPath", alias = "ws_path")]
    pub ws_path: String,

    #[serde(default)]
    pub tls: ClientTlsConfig,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            protocol: default_protocol(),
            pool_count: default_pool_count(),
            tcp_mux: default_tcp_mux(),
            mux_keepalive_secs: default_mux_keepalive_secs(),
            heartbeat_interval: default_heartbeat_interval(),
            heartbeat_timeout: default_heartbeat_timeout(),
            quic: QuicOptions::default(),
            ws_path: default_ws_path(),
            tls: ClientTlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTlsConfig {
    #[serde(default = "default_tls_enable")]
    pub enable: bool,
    #[serde(default, rename = "certFile", alias = "cert_file")]
    pub cert_file: String,
    #[serde(default, rename = "keyFile", alias = "key_file")]
    pub key_file: String,
    #[serde(default, rename = "trustedCaFile", alias = "trusted_ca_file")]
    pub trusted_ca_file: String,
    #[serde(default, rename = "serverName", alias = "server_name")]
    pub server_name: String,
}

impl Default for ClientTlsConfig {
    fn default() -> Self {
        Self {
            enable: default_tls_enable(),
            cert_file: String::new(),
            key_file: String::new(),
            trusted_ca_file: String::new(),
            server_name: String::new(),
        }
    }
}

fn default_tls_enable() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub name: String,

    pub protocol: String,

    #[serde(default)]
    pub service: String,

    #[serde(default, rename = "remotePort", alias = "remote_port")]
    pub remote_port: u16,

    #[serde(default)]
    pub domains: Vec<String>,

    #[serde(default)]
    pub locations: Vec<String>,

    #[serde(default, rename = "basicAuthUser", alias = "basic_auth_user")]
    pub basic_auth_user: String,
    #[serde(default, rename = "basicAuthPassword", alias = "basic_auth_password")]
    pub basic_auth_password: String,

    #[serde(default, rename = "hostHeaderRewrite", alias = "host_header_rewrite")]
    pub host_header_rewrite: String,

    #[serde(default)]
    pub transport: TunnelTransportConfig,

    #[serde(default)]
    pub plugin: Option<PluginConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginConfig {
    #[serde(rename = "type", alias = "plugin_type")]
    pub plugin_type: String,

    #[serde(default)]
    pub service: String,

    #[serde(default, rename = "certFile", alias = "cert_file")]
    pub cert_file: String,
    #[serde(default, rename = "keyFile", alias = "key_file")]
    pub key_file: String,
    #[serde(default, rename = "hostHeaderRewrite", alias = "host_header_rewrite")]
    pub host_header_rewrite: String,

    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TunnelTransportConfig {
    #[serde(default)]
    pub bandwidth: f64,

    #[serde(
        default = "default_bandwidth_limit_side",
        rename = "bandwidthLimitSide",
        alias = "bandwidth_limit_side"
    )]
    pub bandwidth_limit_side: String,

    #[serde(
        default,
        rename = "proxyProtocolVersion",
        alias = "proxy_protocol_version"
    )]
    pub proxy_protocol_version: String,
}

fn default_bandwidth_limit_side() -> String {
    "client".into()
}

fn default_auth_type() -> String {
    "token".into()
}

fn default_protocol() -> String {
    "tcp".into()
}

fn default_pool_count() -> i32 {
    1
}

fn default_tcp_mux() -> bool {
    true
}

fn default_ws_path() -> String {
    crate::transport::ORBIEN_WEBSOCKET_PATH.to_string()
}

fn default_mux_keepalive_secs() -> i64 {
    30
}

fn default_heartbeat_interval() -> i64 {
    -1
}

fn default_heartbeat_timeout() -> i64 {
    -1
}

fn default_udp_packet_size() -> usize {
    1500
}

fn default_server() -> String {
    "127.0.0.1:9527".into()
}

impl TunnelConfig {
    pub fn service_host_port(&self) -> anyhow::Result<(String, u16)> {
        let raw = self.service.trim();
        if raw.is_empty() {
            return Ok(("127.0.0.1".into(), 0));
        }
        let (host, port) = parse_host_port(raw, 0)
            .map_err(|e| anyhow!("tunnel `{}` invalid service: {e}", self.name))?;
        if port == 0 {
            return Err(anyhow!(
                "tunnel `{}` service must include a port (got {raw:?})",
                self.name
            ));
        }
        if host.is_empty() {
            return Err(anyhow!("tunnel `{}` service has empty host", self.name));
        }
        Ok((host, port))
    }

    pub fn has_plugin(&self) -> bool {
        matches!(
            self.plugin
                .as_ref()
                .map(|p| p.plugin_type.trim().is_empty()),
            Some(false)
        )
    }

    pub fn requires_local_service(&self) -> bool {
        !self.has_plugin()
    }
}

enum LoadMode {
    Runtime,
    Edit,
}

impl ClientConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::load_with(path, LoadMode::Runtime)
    }

    pub fn load_for_edit(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::load_with(path, LoadMode::Edit)
    }

    fn load_with(path: impl AsRef<Path>, mode: LoadMode) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let file = super::read_toml_file(path)?;
        let expanded = match mode {
            LoadMode::Runtime => Some(super::expand_env_placeholders(&file)?),
            LoadMode::Edit => {
                super::env::reject_env_placeholders(&file)?;
                None
            }
        };
        let text = expanded.as_deref().unwrap_or(file.as_str());
        let mut cfg: Self = toml::from_str(text)
            .with_context(|| format!("failed to parse config file '{}'", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        if matches!(mode, LoadMode::Runtime) {
            cfg.resolve_paths(base);
        }
        cfg.complete();
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn prepare_runtime(&mut self, config_path: &Path) {
        let base = config_path.parent().unwrap_or_else(|| Path::new("."));
        self.resolve_paths(base);
        self.complete();
    }

    fn resolve_paths(&mut self, base: &Path) {
        let tls = &mut self.transport.tls;
        tls.cert_file = super::resolve_maybe_relative(base, &tls.cert_file);
        tls.key_file = super::resolve_maybe_relative(base, &tls.key_file);
        tls.trusted_ca_file = super::resolve_maybe_relative(base, &tls.trusted_ca_file);
        for t in &mut self.tunnels {
            if let Some(ref mut plugin) = t.plugin {
                plugin.cert_file = super::resolve_maybe_relative(base, &plugin.cert_file);
                plugin.key_file = super::resolve_maybe_relative(base, &plugin.key_file);
            }
        }
    }

    pub fn complete(&mut self) {
        if matches!(self.protocol().ok(), Some(crate::transport::Protocol::Quic)) {
            self.transport.tcp_mux = false;
            self.transport.tls.enable = true;
        }

        if !self.transport.tcp_mux {
            if self.transport.heartbeat_interval < 0 {
                self.transport.heartbeat_interval = 30;
            }
            if self.transport.heartbeat_timeout < 0 {
                self.transport.heartbeat_timeout = 90;
            }
        }

        if self.auth.auth_type.trim().is_empty() {
            self.auth.auth_type = default_auth_type();
        }

        if self.udp_packet_size == 0 {
            self.udp_packet_size = default_udp_packet_size();
        }

        if self.transport.tls.server_name.trim().is_empty() {
            if let Ok(host) = self.server_host() {
                self.transport.tls.server_name = host;
            }
        }

        self.transport.ws_path = crate::transport::normalize_ws_path(&self.transport.ws_path);
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.server.trim().is_empty() {
            return Err(anyhow!("server is required (host:port)"));
        }
        let (host, port) = parse_host_port(&self.server, 9527)
            .map_err(|e| anyhow!("invalid server {:?}: {e}", self.server))?;
        if host.is_empty() {
            return Err(anyhow!("server host is empty"));
        }
        if port == 0 {
            return Err(anyhow!("server port must be > 0"));
        }
        let _ = self.protocol()?;
        if self.transport.pool_count < 1 {
            return Err(anyhow!(
                "transport.poolCount must be >= 1, got {}",
                self.transport.pool_count
            ));
        }
        if self.udp_packet_size == 0 || self.udp_packet_size > 65535 {
            return Err(anyhow!(
                "udpPacketSize out of range: {}",
                self.udp_packet_size
            ));
        }
        if self.transport.heartbeat_timeout > 0
            && self.transport.heartbeat_interval > 0
            && self.transport.heartbeat_timeout < self.transport.heartbeat_interval
        {
            return Err(anyhow!(
                "heartbeatTimeout ({}) must be >= heartbeatInterval ({})",
                self.transport.heartbeat_timeout,
                self.transport.heartbeat_interval
            ));
        }
        let mut seen_names = std::collections::HashSet::new();
        for t in &self.tunnels {
            if t.name.trim().is_empty() {
                return Err(anyhow!("tunnel name is required"));
            }
            if !seen_names.insert(t.name.clone()) {
                return Err(anyhow!("duplicate tunnel name `{}`", t.name));
            }
            let proto = t.protocol.trim().to_ascii_lowercase();
            if !matches!(proto.as_str(), "tcp" | "udp" | "http" | "https") {
                return Err(anyhow!(
                    "tunnel `{}` unsupported protocol {:?}",
                    t.name,
                    t.protocol
                ));
            }
            if !t.transport.bandwidth.is_finite() || t.transport.bandwidth < 0.0 {
                return Err(anyhow!(
                    "tunnel `{}` invalid bandwidth {}",
                    t.name,
                    t.transport.bandwidth
                ));
            }
            let side = t.transport.bandwidth_limit_side.trim().to_ascii_lowercase();
            if !side.is_empty() && side != "client" && side != "server" {
                return Err(anyhow!(
                    "tunnel `{}` invalid bandwidthLimitSide {:?}",
                    t.name,
                    t.transport.bandwidth_limit_side
                ));
            }
            match proto.as_str() {
                "tcp" | "udp" => {
                    if t.remote_port == 0 {
                        return Err(anyhow!(
                            "tunnel `{}` remotePort is required for {}",
                            t.name,
                            proto
                        ));
                    }
                    if t.requires_local_service() {
                        let _ = t.service_host_port()?;
                    }
                    if let Some(plugin) = &t.plugin {
                        Self::validate_tcp_tunnel_plugin(t.name.as_str(), plugin)?;
                    }
                }
                "http" | "https" => {
                    if t.domains.is_empty() {
                        return Err(anyhow!(
                            "tunnel `{}` domains is required for {}",
                            t.name,
                            proto
                        ));
                    }
                    if t.requires_local_service() {
                        let _ = t.service_host_port()?;
                    }
                    if let Some(plugin) = &t.plugin {
                        Self::validate_https_tunnel_plugin(t.name.as_str(), plugin)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_tcp_tunnel_plugin(name: &str, plugin: &PluginConfig) -> anyhow::Result<()> {
        let pt = plugin.plugin_type.trim().to_ascii_lowercase();
        if pt.is_empty() {
            return Ok(());
        }
        match pt.as_str() {
            "socks5" => Self::validate_socks5_plugin_fields(name, plugin),
            other => Err(anyhow!(
                "tunnel `{}` unsupported plugin.type {:?}",
                name,
                other
            )),
        }
    }

    fn validate_https_tunnel_plugin(name: &str, plugin: &PluginConfig) -> anyhow::Result<()> {
        let pt = plugin.plugin_type.trim().to_ascii_lowercase();
        if pt.is_empty() {
            return Ok(());
        }
        match pt.as_str() {
            "tls-term" => {
                if plugin.service.trim().is_empty() {
                    return Err(anyhow!(
                        "tunnel `{}` plugin.service is required for tls-term",
                        name
                    ));
                }
                let (h, p) = parse_host_port(&plugin.service, 0)
                    .map_err(|e| anyhow!("tunnel `{}` invalid plugin.service: {e}", name))?;
                if h.is_empty() || p == 0 {
                    return Err(anyhow!(
                        "tunnel `{}` plugin.service must be host:port",
                        name
                    ));
                }
                Ok(())
            }
            other => Err(anyhow!(
                "tunnel `{}` unsupported plugin.type {:?}",
                name,
                other
            )),
        }
    }

    fn validate_socks5_plugin_fields(name: &str, plugin: &PluginConfig) -> anyhow::Result<()> {
        let user = plugin.username.trim();
        let pass = plugin.password.trim();
        if user.is_empty() || pass.is_empty() {
            return Err(anyhow!(
                "tunnel `{}` socks5 requires username and password",
                name
            ));
        }
        Ok(())
    }

    pub fn server_host(&self) -> anyhow::Result<String> {
        Ok(parse_host_port(&self.server, 9527)?.0)
    }

    pub fn server_port(&self) -> anyhow::Result<u16> {
        Ok(parse_host_port(&self.server, 9527)?.1)
    }

    pub fn tls_server_name(&self) -> String {
        let sn = self.transport.tls.server_name.trim();
        if sn.is_empty() {
            self.server_host().unwrap_or_else(|_| "localhost".into())
        } else {
            sn.to_string()
        }
    }

    pub fn server_endpoint(&self) -> String {
        self.server.trim().to_string()
    }

    pub fn protocol(&self) -> anyhow::Result<crate::transport::Protocol> {
        crate::transport::Protocol::parse(&self.transport.protocol).ok_or_else(|| {
            anyhow::anyhow!(
                "unsupported transport.protocol {:?}, use tcp|quic|websocket|kcp",
                self.transport.protocol
            )
        })
    }

    pub fn uses_yamux(&self) -> bool {
        self.transport.tcp_mux
            && matches!(
                self.protocol().ok(),
                Some(
                    crate::transport::Protocol::Tcp
                        | crate::transport::Protocol::Websocket
                        | crate::transport::Protocol::Kcp
                )
            )
    }

    pub fn connection_settings_eq(&self, other: &Self) -> bool {
        self.server == other.server
            && self.user == other.user
            && self.agent_id == other.agent_id
            && self.auth == other.auth
            && self.transport == other.transport
            && self.udp_packet_size == other.udp_packet_size
    }
}
