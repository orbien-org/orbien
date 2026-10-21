use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct PortTable {
    by_port: Mutex<HashMap<u16, String>>,
}

impl PortTable {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn claim(&self, port: u16, tunnel_name: &str) -> Result<()> {
        if port == 0 {
            return Err(anyhow!("invalid remote port 0"));
        }
        let mut map = self.by_port.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&port) {
            return Err(anyhow!(
                "remote port {port} is already in use by tunnel `{existing}`"
            ));
        }
        map.insert(port, tunnel_name.to_string());
        Ok(())
    }

    pub fn release(&self, port: u16, tunnel_name: &str) {
        let mut map = self.by_port.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(&port).map(|n| n == tunnel_name).unwrap_or(false) {
            map.remove(&port);
        }
    }

    /// Drop every port claimed by a tunnel name.
    pub fn release_name(&self, tunnel_name: &str) {
        let mut map = self.by_port.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, name| name != tunnel_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_name_frees_only_matching_tunnel() {
        let ports = PortTable::new();
        ports.claim(9001, "a").unwrap();
        ports.claim(9002, "b").unwrap();

        ports.release_name("a");
        ports.claim(9001, "c").unwrap();
        assert!(ports.claim(9002, "c").is_err());
    }
}
