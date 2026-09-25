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
}
