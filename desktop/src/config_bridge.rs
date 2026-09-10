use anyhow::{anyhow, Context, Result};
use orbien_client::ClientConfig;
use orbien_core::config::{
    parse_host_port, ClientTlsConfig, PluginConfig, TransportConfig, TunnelConfig,
    TunnelTransportConfig,
};
use std::fs;
use std::path::{Path, PathBuf};

pub fn default_config_path() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(target_os = "macos")]
    {
        let dir = base.join("Library/Application Support/com.orbien.desktop");
        let _ = fs::create_dir_all(&dir);
        return dir.join("orbien.toml");
    }
    #[cfg(not(target_os = "macos"))]
    {
        let dir = base.join(".config").join("orbien");
        let _ = fs::create_dir_all(&dir);
        dir.join("orbien.toml")
    }
}

pub fn resolve_path(config_path: &str) -> PathBuf {
    let trimmed = config_path.trim();
    if trimmed.is_empty() {
        default_config_path()
    } else {
        PathBuf::from(trimmed)
    }
}

pub fn path_display(path: &Path) -> String {
    path.display().to_string()
}

pub fn split_server_endpoint(server: &str) -> (String, String) {
    match parse_host_port(server, 9527) {
        Ok((host, port)) => (host, port.to_string()),
        Err(_) => {
            let t = server.trim();
            if t.is_empty() {
                ("127.0.0.1".into(), "9527".into())
            } else {
                (t.to_string(), "9527".into())
            }
        }
    }
}

pub fn load_config(config_path: &str) -> Result<(ClientConfig, PathBuf)> {
    let path = resolve_path(config_path);
    let mut cfg =
        ClientConfig::load_for_edit(&path).with_context(|| format!("load {}", path.display()))?;
    if ensure_agent_id(&mut cfg) {
        save(&path, &cfg)?;
    }
    Ok((cfg, path))
}

pub fn load_merge_tunnels(
    config_path: &str,
    server_addr: &str,
    server_port: &str,
    token: &str,
    user: &str,
    protocol_index: i32,
    pool_count: &str,
    tcp_mux: bool,
    tls_enable: bool,
    tunnels: Vec<TunnelConfig>,
) -> Result<(ClientConfig, PathBuf)> {
    let path = resolve_path(config_path);
    let mut cfg = if path.is_file() {
        ClientConfig::load_for_edit(&path).with_context(|| format!("load {}", path.display()))?
    } else {
        build_base(
            server_addr,
            server_port,
            token,
            user,
            protocol_index,
            pool_count,
            tcp_mux,
            tls_enable,
            Vec::new(),
        )?
    };
    cfg.tunnels = tunnels;
    cfg.complete();
    let _ = ensure_agent_id(&mut cfg);
    Ok((cfg, path))
}

pub fn load_merge_server_fields(
    config_path: &str,
    server_addr: &str,
    server_port: &str,
    token: &str,
    user: &str,
    protocol_index: i32,
    pool_count: &str,
    tcp_mux: bool,
    tls_enable: bool,
    ws_path: &str,
    mux_keepalive: &str,
    heartbeat_interval: &str,
    heartbeat_timeout: &str,
    udp_packet_size: &str,
    tls_server_name: &str,
    tls_ca: &str,
    tls_cert: &str,
    tls_key: &str,
    quic_keepalive: &str,
    quic_idle: &str,
    quic_streams: &str,
    tunnels: Vec<TunnelConfig>,
) -> Result<(ClientConfig, PathBuf)> {
    let path = resolve_path(config_path);
    let mut cfg = if path.is_file() {
        ClientConfig::load_for_edit(&path).with_context(|| format!("load {}", path.display()))?
    } else {
        ClientConfig {
            server: String::new(),
            user: String::new(),
            agent_id: String::new(),
            auth: Default::default(),
            transport: TransportConfig::default(),
            tunnels: Vec::new(),
            udp_packet_size: 1500,
        }
    };

    cfg.server = assemble_server(server_addr, server_port)?;
    cfg.user = user.trim().into();
    cfg.auth.auth_type = "token".into();
    cfg.auth.token = token.to_string();

    let protocols = ["tcp", "websocket", "quic", "kcp"];
    let protocol = protocols
        .get(protocol_index as usize)
        .copied()
        .unwrap_or("tcp")
        .to_string();

    cfg.transport.protocol = protocol;
    cfg.transport.pool_count = pool_count.trim().parse().unwrap_or(1);
    let is_quic = protocol_index == 2;
    cfg.transport.tcp_mux = !is_quic && tcp_mux;
    cfg.transport.ws_path = ws_path.trim().into();
    cfg.transport.mux_keepalive_secs = mux_keepalive.trim().parse().unwrap_or(30);
    cfg.transport.heartbeat_interval = parse_optional_i64(heartbeat_interval, -1);
    cfg.transport.heartbeat_timeout = parse_optional_i64(heartbeat_timeout, -1);
    cfg.udp_packet_size = udp_packet_size.trim().parse().unwrap_or(1500);
    cfg.transport.tls.enable = is_quic || tls_enable;
    cfg.transport.tls.server_name = tls_server_name.trim().into();
    cfg.transport.tls.trusted_ca_file = tls_ca.trim().into();
    cfg.transport.tls.cert_file = tls_cert.trim().into();
    cfg.transport.tls.key_file = tls_key.trim().into();
    cfg.transport.quic.keepalive_period = quic_keepalive.trim().parse().unwrap_or(10);
    cfg.transport.quic.max_idle_timeout = quic_idle.trim().parse().unwrap_or(30);
    cfg.transport.quic.max_incoming_streams = quic_streams.trim().parse().unwrap_or(100_000);
    cfg.tunnels = tunnels;
    cfg.complete();

    if cfg.transport.heartbeat_timeout > 0
        && cfg.transport.heartbeat_interval > 0
        && cfg.transport.heartbeat_timeout < cfg.transport.heartbeat_interval
    {
        return Err(anyhow!(
            "heartbeatTimeout ({}) must be >= heartbeatInterval ({})",
            cfg.transport.heartbeat_timeout,
            cfg.transport.heartbeat_interval
        ));
    }

    let _ = ensure_agent_id(&mut cfg);
    Ok((cfg, path))
}

pub fn save(path: &Path, cfg: &ClientConfig) -> Result<()> {
    ensure_parent(path)?;
    let body = toml::to_string_pretty(cfg).context("serialize client config")?;
    fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn ensure_agent_id(cfg: &mut ClientConfig) -> bool {
    if !cfg.agent_id.trim().is_empty() {
        return false;
    }
    cfg.agent_id = new_agent_id();
    true
}

fn new_agent_id() -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string();
    hex[..19].to_owned()
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("create dir {}", parent.display()))?;
    }
    Ok(())
}

fn build_base(
    server_addr: &str,
    server_port: &str,
    token: &str,
    user: &str,
    protocol_index: i32,
    pool_count: &str,
    tcp_mux: bool,
    tls_enable: bool,
    tunnels: Vec<TunnelConfig>,
) -> Result<ClientConfig> {
    let server = assemble_server(server_addr, server_port)?;
    let protocols = ["tcp", "websocket", "quic", "kcp"];
    let protocol = protocols
        .get(protocol_index as usize)
        .copied()
        .unwrap_or("tcp")
        .to_string();
    let pool: i32 = pool_count.trim().parse().unwrap_or(1);
    let is_quic = protocol_index == 2;
    let mut transport = TransportConfig::default();
    transport.protocol = protocol;
    transport.pool_count = pool;
    transport.tcp_mux = !is_quic && tcp_mux;
    transport.tls = ClientTlsConfig {
        enable: is_quic || tls_enable,
        ..ClientTlsConfig::default()
    };
    let mut cfg = ClientConfig {
        server,
        user: user.trim().into(),
        agent_id: new_agent_id(),
        auth: Default::default(),
        transport,
        tunnels,
        udp_packet_size: 1500,
    };
    cfg.auth.auth_type = "token".into();
    cfg.auth.token = token.to_string();
    cfg.complete();
    Ok(cfg)
}

fn assemble_server(server_addr: &str, server_port: &str) -> Result<String> {
    let addr = server_addr.trim();
    if addr.is_empty() {
        return Err(anyhow!("server address is required"));
    }
    let port: u16 = server_port
        .trim()
        .parse()
        .map_err(|_| anyhow!("invalid server port: {server_port}"))?;
    Ok(format!("{addr}:{port}"))
}

fn assemble_service(local_ip: &str, local_port: &str) -> Result<String> {
    let ip = local_ip.trim();
    let port_raw = local_port.trim();
    if ip.is_empty() && port_raw.is_empty() {
        return Ok(String::new());
    }
    let host = if ip.is_empty() { "127.0.0.1" } else { ip };
    let port = parse_u16(port_raw, "service port")?;
    Ok(format!("{host}:{port}"))
}

fn parse_optional_i64(raw: &str, default: i64) -> i64 {
    let t = raw.trim();
    if t.is_empty() {
        default
    } else {
        t.parse().unwrap_or(default)
    }
}

fn parse_u16(raw: &str, field: &str) -> Result<u16> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(0);
    }
    t.parse().map_err(|_| anyhow!("invalid {field}: {raw}"))
}

fn parse_bandwidth_mbps(raw: &str) -> f64 {
    let t = raw.trim();
    if t.is_empty() {
        0.0
    } else {
        t.parse().unwrap_or(0.0)
    }
}

fn bandwidth_display(v: f64) -> String {
    if v == 0.0 {
        String::new()
    } else if v.fract() == 0.0 && v.abs() < (i64::MAX as f64) {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c == ';' || c == '\n')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_tls_term_plugin(plugin_type: &str) -> bool {
    matches!(plugin_type.trim().to_ascii_lowercase().as_str(), "tls-term")
}

fn is_socks5_plugin(plugin_type: &str) -> bool {
    matches!(plugin_type.trim().to_ascii_lowercase().as_str(), "socks5")
}

pub fn tunnel_from_parts(
    name: &str,
    tunnel_type: &str,
    local_ip: &str,
    local_port: &str,
    remote_port: &str,
    domains: &str,
    locations: &str,
    basic_auth_user: &str,
    basic_auth_password: &str,
    host_header_rewrite: &str,
    bandwidth: &str,
    bandwidth_limit_side: &str,
    proxy_protocol_version: &str,
    plugin_tls_term: bool,
    plugin_local_addr: &str,
    plugin_cert_file: &str,
    plugin_key_file: &str,
    plugin_host_rewrite: &str,
    plugin_username: &str,
    plugin_password: &str,
) -> Result<TunnelConfig> {
    let ty = tunnel_type.trim().to_ascii_lowercase();
    let plugin = if ty == "socks5" {
        Some(PluginConfig {
            plugin_type: "socks5".into(),
            username: plugin_username.trim().into(),
            password: plugin_password.trim().into(),
            ..Default::default()
        })
    } else if ty == "https" && plugin_tls_term {
        Some(PluginConfig {
            plugin_type: "tls-term".into(),
            service: plugin_local_addr.trim().into(),
            cert_file: plugin_cert_file.trim().into(),
            key_file: plugin_key_file.trim().into(),
            host_header_rewrite: plugin_host_rewrite.trim().into(),
            username: String::new(),
            password: String::new(),
        })
    } else {
        None
    };

    let protocol = if ty == "socks5" {
        "tcp".into()
    } else {
        ty.clone()
    };

    let service = if plugin.is_some() {
        String::new()
    } else {
        assemble_service(local_ip, local_port)?
    };

    Ok(TunnelConfig {
        name: name.trim().into(),
        protocol,
        service,
        remote_port: parse_u16(remote_port, "remotePort")?,
        domains: split_csv(domains),
        locations: split_csv(locations),
        basic_auth_user: basic_auth_user.trim().into(),
        basic_auth_password: basic_auth_password.trim().into(),
        host_header_rewrite: host_header_rewrite.trim().into(),
        transport: TunnelTransportConfig {
            bandwidth: parse_bandwidth_mbps(bandwidth),
            bandwidth_limit_side: if bandwidth_limit_side.trim() == "server" {
                "server".into()
            } else {
                "client".into()
            },
            proxy_protocol_version: proxy_protocol_version.trim().into(),
        },
        plugin,
    })
}

pub fn tunnel_to_parts(p: &TunnelConfig) -> TunnelParts {
    let socks5 = p
        .plugin
        .as_ref()
        .filter(|pl| is_socks5_plugin(&pl.plugin_type));
    let tls_term = p
        .plugin
        .as_ref()
        .filter(|pl| is_tls_term_plugin(&pl.plugin_type));

    let tunnel_type = if socks5.is_some() {
        "socks5".into()
    } else {
        p.protocol.clone()
    };

    let (local_ip, local_port) = if socks5.is_some() {
        (String::new(), String::new())
    } else if p.service.trim().is_empty() {
        ("127.0.0.1".into(), "0".into())
    } else {
        match parse_host_port(&p.service, 0) {
            Ok((host, port)) => (host, port.to_string()),
            Err(_) => (p.service.clone(), "0".into()),
        }
    };

    TunnelParts {
        name: p.name.clone(),
        tunnel_type,
        local_ip,
        local_port,
        remote_port: p.remote_port.to_string(),
        domains: p.domains.join(","),
        locations: p.locations.join(","),
        basic_auth_user: p.basic_auth_user.clone(),
        basic_auth_password: p.basic_auth_password.clone(),
        host_header_rewrite: p.host_header_rewrite.clone(),
        bandwidth: bandwidth_display(p.transport.bandwidth),
        bandwidth_limit_side: p.transport.bandwidth_limit_side.clone(),
        proxy_protocol_version: p.transport.proxy_protocol_version.clone(),
        plugin_tls_term: tls_term.is_some(),
        plugin_local_addr: tls_term
            .map(|pl| pl.service.clone())
            .unwrap_or_else(|| "127.0.0.1:80".into()),
        plugin_cert_file: tls_term.map(|pl| pl.cert_file.clone()).unwrap_or_default(),
        plugin_key_file: tls_term.map(|pl| pl.key_file.clone()).unwrap_or_default(),
        plugin_host_rewrite: tls_term
            .map(|pl| pl.host_header_rewrite.clone())
            .unwrap_or_default(),
        plugin_username: socks5.map(|pl| pl.username.clone()).unwrap_or_default(),
        plugin_password: socks5.map(|pl| pl.password.clone()).unwrap_or_default(),
    }
}

#[derive(Debug, Clone)]
pub struct TunnelParts {
    pub name: String,
    pub tunnel_type: String,
    pub local_ip: String,
    pub local_port: String,
    pub remote_port: String,
    pub domains: String,
    pub locations: String,
    pub basic_auth_user: String,
    pub basic_auth_password: String,
    pub host_header_rewrite: String,
    pub bandwidth: String,
    pub bandwidth_limit_side: String,
    pub proxy_protocol_version: String,
    pub plugin_tls_term: bool,
    pub plugin_local_addr: String,
    pub plugin_cert_file: String,
    pub plugin_key_file: String,
    pub plugin_host_rewrite: String,
    pub plugin_username: String,
    pub plugin_password: String,
}

pub fn protocol_index(protocol: &str) -> i32 {
    match protocol.trim().to_ascii_lowercase().as_str() {
        "websocket" | "ws" => 1,
        "quic" => 2,
        "kcp" => 3,
        _ => 0,
    }
}

pub fn optional_i64_display(v: i64) -> String {
    if v < 0 {
        String::new()
    } else {
        v.to_string()
    }
}

pub fn ws_path_display(path: &str) -> String {
    let t = path.trim();
    if t.is_empty() || t == orbien_core::transport::ORBIEN_WEBSOCKET_PATH {
        String::new()
    } else {
        t.to_string()
    }
}
