mod client;
mod env;
mod server;

pub use client::{
    ClientConfig, ClientTlsConfig, PluginConfig, TransportConfig, TunnelConfig,
    TunnelTransportConfig,
};
pub use env::{contains_env_placeholders, expand_env_placeholders};
pub use server::{
    parse_host_port, DashboardConfig, QuicConfig, ServerConfig, ServerTlsConfig,
    ServerTransportConfig,
};

use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};

pub const CLIENT_DEFAULT_CONFIG: &str = "orbien.toml";
pub const CLIENT_CONFIG_CANDIDATES: &[&str] = &["orbien.toml", "conf/orbien.toml"];

pub fn resolve_client_config_path(explicit: Option<&str>) -> anyhow::Result<PathBuf> {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Ok(path);
        }
        anyhow::bail!(
            "config file not found: '{raw}'\n\
             \n\
             Specify a valid TOML with -c/--config.\n\
             Example:\n\
               orbien -c conf/orbien.toml"
        );
    }

    for cand in CLIENT_CONFIG_CANDIDATES {
        let path = PathBuf::from(cand);
        if path.is_file() {
            return Ok(path);
        }
    }

    let tried = CLIENT_CONFIG_CANDIDATES.join(", ");
    anyhow::bail!(
        "no config file found (tried: {tried})\n\
         \n\
         A config file is required.\n\
         Specify one with -c/--config.\n\
         Example:\n\
           orbien -c conf/orbien.toml"
    )
}

pub(crate) fn read_toml_file(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read config file '{}'; specify a valid path with -c/--config",
            path.display()
        )
    })
}

pub(crate) fn resolve_maybe_relative(base: &Path, p: &str) -> String {
    let p = p.trim();
    if p.is_empty() {
        return String::new();
    }
    let path = Path::new(p);
    if path.is_absolute() {
        p.to_string()
    } else {
        base.join(path)
            .canonicalize()
            .unwrap_or_else(|_| base.join(path))
            .to_string_lossy()
            .into_owned()
    }
}
