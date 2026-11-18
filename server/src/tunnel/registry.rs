use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Mutex;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelOwner {
    pub session_id: String,
    pub generation: u64,
}
#[derive(Debug, Default)]
pub struct TunnelRegistry {
    by_name: Mutex<HashMap<String, TunnelOwner>>,
}
impl TunnelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_insert(&self, name: &str, owner: TunnelOwner) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("empty tunnel name"));
        }
        let mut map = self.by_name.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(name) {
            return Err(anyhow!(
                "tunnel `{name}` is already registered (session={}, generation={})",
                existing.session_id,
                existing.generation
            ));
        }
        map.insert(name.to_string(), owner);
        Ok(())
    }

    pub fn remove_if_owner(&self, name: &str, owner: &TunnelOwner) {
        let mut map = self.by_name.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(name) == Some(owner) {
            map.remove(name);
        }
    }

    /// Remove every name owned by a generation whose control is gone.
    pub fn remove_owner(&self, owner: &TunnelOwner) -> Vec<String> {
        let mut map = self.by_name.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = Vec::new();
        map.retain(|name, existing| {
            if existing == owner {
                removed.push(name.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(session: &str, generation: u64) -> TunnelOwner {
        TunnelOwner {
            session_id: session.into(),
            generation,
        }
    }

    #[test]
    fn remove_owner_only_removes_matching_generation() {
        let reg = TunnelRegistry::new();
        reg.try_insert("a", owner("s1", 1)).unwrap();
        reg.try_insert("b", owner("s1", 2)).unwrap();

        let removed = reg.remove_owner(&owner("s1", 1));
        assert_eq!(removed, vec!["a".to_string()]);
        assert!(reg.try_insert("b", owner("s2", 3)).is_err());
        assert!(reg.try_insert("a", owner("s2", 3)).is_ok());
    }
}
