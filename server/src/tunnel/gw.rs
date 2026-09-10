use crate::control::Control;
use std::collections::HashMap;
use std::sync::{Mutex, Weak};

#[derive(Clone)]
pub struct HttpRoute {
    pub tunnel_name: String,
    pub control: Weak<Control>,
    pub location: String,
    pub host_header_rewrite: String,
    pub basic_auth_user: String,
    pub basic_auth_password: String,
    pub limiter: Option<std::sync::Arc<orbien_core::limit::BandwidthLimiter>>,
}

type DomainIndex = HashMap<String, Vec<HttpRoute>>;

pub struct HttpGw {
    routes: Mutex<DomainIndex>,
    pub listen_port: u16,
}

impl HttpGw {
    pub fn new(listen_port: u16) -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
            listen_port,
        }
    }

    pub fn register(&self, domain: &str, route: HttpRoute) -> anyhow::Result<()> {
        let key = normalize_host(domain);
        if key.is_empty() {
            return Err(anyhow::anyhow!("empty http domain"));
        }
        let mut map = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        let list = map.entry(key.clone()).or_default();

        if let Some(existing) = list.iter().find(|r| r.location == route.location) {
            if existing.tunnel_name != route.tunnel_name {
                return Err(anyhow::anyhow!(
                    "router config conflict: domain={key} location={}",
                    route.location
                ));
            }
            list.retain(|r| r.location != route.location);
        }

        list.push(route);
        list.sort_by(|a, b| b.location.cmp(&a.location));
        tracing::info!(domain = %key, "http route registered");
        Ok(())
    }

    pub fn unregister_tunnel(&self, tunnel_name: &str) {
        let mut map = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, list| {
            list.retain(|r| r.tunnel_name != tunnel_name);
            !list.is_empty()
        });
    }

    pub fn lookup(&self, host: &str, path: &str) -> Option<HttpRoute> {
        let key = normalize_host(host);
        let map = self.routes.lock().unwrap_or_else(|e| e.into_inner());
        match_location(&map, &key, path).cloned()
    }
}

fn match_location<'a>(map: &'a DomainIndex, host: &str, path: &str) -> Option<&'a HttpRoute> {
    let list = map.get(host)?;
    for route in list {
        if path.starts_with(&route.location) {
            return Some(route);
        }
    }
    None
}

pub fn normalize_host(host: &str) -> String {
    let host = host.trim();
    let without_port = if let Some(h) = host.strip_prefix('[') {
        if let Some(end) = h.find(']') {
            &h[..end]
        } else {
            host
        }
    } else {
        host.split(':').next().unwrap_or(host)
    };
    without_port
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

pub fn build_domains(domains: &[String], root_domain: &str) -> anyhow::Result<Vec<String>> {
    let root = normalize_host(root_domain);
    let entries: Vec<String> = domains
        .iter()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect();

    if entries.is_empty() {
        return Err(anyhow::anyhow!(
            "http/https requires at least one domain (fullDomain or subdomain prefix)"
        ));
    }

    let mut out = Vec::new();
    for entry in &entries {
        out.push(expand_domain_entry(entry, &root)?);
    }

    let mut seen = std::collections::HashSet::new();
    out.retain(|d| seen.insert(d.clone()));
    Ok(out)
}

fn expand_domain_entry(entry: &str, root: &str) -> anyhow::Result<String> {
    let e = normalize_host(entry);
    if e.is_empty() {
        return Err(anyhow::anyhow!("empty domain entry"));
    }
    if e.starts_with('.') || e.ends_with('.') || e.contains("..") {
        return Err(anyhow::anyhow!("invalid domain entry: {entry:?}"));
    }

    if !e.contains('.') {
        if e.contains('*') {
            return Err(anyhow::anyhow!(
                "subdomain prefix must not contain '*': {entry:?}"
            ));
        }
        if e.len() > 63 {
            return Err(anyhow::anyhow!(
                "subdomain prefix too long (max 63): {entry:?}"
            ));
        }
        if root.is_empty() {
            return Err(anyhow::anyhow!(
                "domain prefix {entry:?} requires server rootDomain"
            ));
        }
        return Ok(normalize_host(&format!("{e}.{root}")));
    }

    if e.contains('*') {
        return Err(anyhow::anyhow!(
            "wildcard domains are not supported: {entry:?}"
        ));
    }

    if !root.is_empty() && is_host_under_root(&e, root) {
        return Ok(e);
    }

    Ok(e)
}

fn is_host_under_root(host: &str, root: &str) -> bool {
    host == root || host.ends_with(&format!(".{root}"))
}

pub fn expand_locations(locations: &[String]) -> Vec<String> {
    if locations.is_empty() {
        vec![String::new()]
    } else {
        locations.to_vec()
    }
}

pub fn parse_basic_auth(header_value: &str) -> Option<(String, String)> {
    const PREFIX: &str = "Basic ";
    let value = header_value.trim();
    if value.len() < PREFIX.len() || !value[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value[PREFIX.len()..].trim())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, pass) = text.split_once(':')?;
    Some((user.to_string(), pass.to_string()))
}

pub fn route_basic_auth_ok(
    route: &HttpRoute,
    is_proxy_request: bool,
    authorization: Option<&str>,
    proxy_authorization: Option<&str>,
) -> bool {
    if route.basic_auth_user.is_empty() && route.basic_auth_password.is_empty() {
        return true;
    }
    let creds = if is_proxy_request {
        let Some(h) = proxy_authorization else {
            return false;
        };
        parse_basic_auth(h)
    } else {
        let Some(h) = authorization else {
            return false;
        };
        parse_basic_auth(h)
    };
    match creds {
        Some((u, p)) => u == route.basic_auth_user && p == route.basic_auth_password,
        None => false,
    }
}
