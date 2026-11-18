use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

pub const MAX_AGENT_ID_LEN: usize = 64;
pub const MAX_SESSION_ID_LEN: usize = 64;
pub const MAX_USER_LEN: usize = 64;

#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub user: String,
    pub agent_id: String,
    pub session_id: String,
    pub generation: u64,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub client_ip: String,
    pub version: String,
    pub tunnel_count: usize,
    pub online: bool,
    pub disconnected_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct AgentOnlineSpec {
    pub user: String,
    pub agent_id: String,
    pub session_id: String,
    pub generation: u64,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub client_ip: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConflict {
    pub user: String,
    pub agent_id: String,
    pub session_id: String,
    pub generation: u64,
    pub tunnel_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRegisterError {
    Conflict(AgentConflict),
}

#[derive(Debug, Default)]
pub struct AgentRegistry {
    by_key: Mutex<HashMap<String, AgentEntry>>,
    by_session: Mutex<HashMap<String, String>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_online(&self, spec: AgentOnlineSpec) -> Result<String, AgentRegisterError> {
        let now = Instant::now();
        let explicit = !spec.agent_id.is_empty();
        let effective_id = if explicit {
            spec.agent_id.as_str()
        } else {
            spec.session_id.as_str()
        };
        let key = compose_key(&spec.user, effective_id);

        let mut by_key = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        let mut by_session = self.by_session.lock().unwrap_or_else(|e| e.into_inner());

        if explicit {
            if let Some(existing) = by_key.get(&key) {
                if existing.online
                    && !existing.session_id.is_empty()
                    && existing.session_id != spec.session_id
                {
                    return Err(AgentRegisterError::Conflict(AgentConflict {
                        user: existing.user.clone(),
                        agent_id: existing.agent_id.clone(),
                        session_id: existing.session_id.clone(),
                        generation: existing.generation,
                        tunnel_count: existing.tunnel_count,
                    }));
                }
            }
        }

        if let Some(prev_key) = by_session.get(&spec.session_id).cloned() {
            if prev_key != key {
                if let Some(prev) = by_key.get_mut(&prev_key) {
                    if prev.session_id == spec.session_id {
                        if prev.agent_id.is_empty() {
                            by_key.remove(&prev_key);
                        } else {
                            set_offline(prev, now, prev.tunnel_count);
                        }
                    }
                }
                by_session.remove(&spec.session_id);
            }
        }

        match by_key.get_mut(&key) {
            Some(entry) => {
                if !entry.session_id.is_empty() && entry.session_id != spec.session_id {
                    by_session.remove(&entry.session_id);
                }
                entry.user = spec.user;
                entry.agent_id = spec.agent_id;
                entry.session_id = spec.session_id.clone();
                entry.generation = spec.generation;
                entry.hostname = spec.hostname;
                entry.os = spec.os;
                entry.arch = spec.arch;
                entry.client_ip = spec.client_ip;
                entry.version = spec.version;
                entry.online = true;
                entry.disconnected_at = None;
            }
            None => {
                by_key.insert(
                    key.clone(),
                    AgentEntry {
                        user: spec.user,
                        agent_id: spec.agent_id,
                        session_id: spec.session_id.clone(),
                        generation: spec.generation,
                        hostname: spec.hostname,
                        os: spec.os,
                        arch: spec.arch,
                        client_ip: spec.client_ip,
                        version: spec.version,
                        tunnel_count: 0,
                        online: true,
                        disconnected_at: None,
                    },
                );
            }
        }

        by_session.insert(spec.session_id, key.clone());
        Ok(key)
    }

    pub fn release(&self, session_id: &str, generation: u64, tunnel_count: usize) {
        let now = Instant::now();
        let mut by_key = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        let mut by_session = self.by_session.lock().unwrap_or_else(|e| e.into_inner());

        let Some(key) = by_session.get(session_id).cloned() else {
            return;
        };
        let Some(entry) = by_key.get_mut(&key) else {
            by_session.remove(session_id);
            return;
        };
        if entry.session_id != session_id || entry.generation != generation {
            return;
        }

        by_session.remove(session_id);
        if entry.agent_id.is_empty() {
            by_key.remove(&key);
        } else {
            set_offline(entry, now, tunnel_count);
        }
    }

    pub fn get(&self, session_id: &str) -> Option<AgentEntry> {
        let by_key = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        let by_session = self.by_session.lock().unwrap_or_else(|e| e.into_inner());
        let key = by_session.get(session_id)?;
        by_key.get(key).cloned()
    }

    /// Clear a leaked online record even when its control teardown did not run.
    ///
    /// `generation` guards against clobbering a newer session that reused the
    /// same session id. Returns true when an online entry was changed.
    pub fn force_release(
        &self,
        session_id: &str,
        generation: Option<u64>,
        tunnel_count: usize,
    ) -> bool {
        let now = Instant::now();
        let mut by_key = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        let mut by_session = self.by_session.lock().unwrap_or_else(|e| e.into_inner());

        let Some(key) = by_session.get(session_id).cloned() else {
            return false;
        };
        let Some(entry) = by_key.get_mut(&key) else {
            by_session.remove(session_id);
            return false;
        };
        if entry.session_id != session_id || !entry.online {
            return false;
        }
        if let Some(generation) = generation {
            if entry.generation != generation {
                return false;
            }
        }

        let anonymous = entry.agent_id.is_empty();
        if !anonymous {
            set_offline(entry, now, tunnel_count);
        }
        by_session.remove(session_id);
        if anonymous {
            by_key.remove(&key);
        }
        true
    }

    pub fn list(&self) -> Vec<AgentEntry> {
        self.by_key
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }
}

fn set_offline(entry: &mut AgentEntry, now: Instant, tunnel_count: usize) {
    entry.generation = 0;
    entry.online = false;
    entry.tunnel_count = tunnel_count;
    entry.disconnected_at = Some(now);
}

fn compose_key(user: &str, id: &str) -> String {
    match (user.is_empty(), id.is_empty()) {
        (true, _) => id.to_string(),
        (_, true) => user.to_string(),
        (false, false) => format!("{user}.{id}"),
    }
}

pub fn sanitize_wire_id(raw: &str, max_len: usize) -> Result<String, &'static str> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    if s.len() > max_len {
        return Err("identifier too long");
    }
    if s.chars().any(|c| c.is_control()) {
        return Err("identifier contains control characters");
    }
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(user: &str, agent_id: &str, session_id: &str, generation: u64) -> AgentOnlineSpec {
        AgentOnlineSpec {
            user: user.into(),
            agent_id: agent_id.into(),
            session_id: session_id.into(),
            generation,
            hostname: "host".into(),
            os: "windows".into(),
            arch: "x86_64".into(),
            client_ip: "127.0.0.1".into(),
            version: "test".into(),
        }
    }

    #[test]
    fn stale_generation_release_leaves_agent_online() {
        let reg = AgentRegistry::new();
        reg.try_online(spec("u", "agent-a", "s1", 1)).unwrap();

        // This is the teardown bug: release is ignored when the generation
        // captured by the old control no longer matches.
        reg.release("s1", 2, 0);

        let entry = reg.get("s1").expect("agent entry");
        assert!(entry.online);
        assert_eq!(entry.generation, 1);

        let err = reg.try_online(spec("u", "agent-a", "s2", 2)).unwrap_err();
        match err {
            AgentRegisterError::Conflict(conflict) => {
                assert_eq!(conflict.session_id, "s1");
                assert_eq!(conflict.generation, 1);
            }
        }
    }

    #[test]
    fn force_release_clears_leaked_record_and_allows_new_session() {
        let reg = AgentRegistry::new();
        reg.try_online(spec("u", "agent-a", "s1", 1)).unwrap();
        reg.release("s1", 2, 0);

        assert!(reg.force_release("s1", Some(1), 0));
        assert!(!reg.force_release("s1", Some(1), 0));

        reg.try_online(spec("u", "agent-a", "s2", 2)).unwrap();
        let entry = reg.get("s2").expect("new session");
        assert!(entry.online);
        assert_eq!(entry.session_id, "s2");
        assert_eq!(entry.generation, 2);
    }

    #[test]
    fn force_release_does_not_clobber_newer_generation() {
        let reg = AgentRegistry::new();
        reg.try_online(spec("u", "agent-a", "s1", 1)).unwrap();

        assert!(!reg.force_release("s1", Some(2), 0));
        assert!(reg.get("s1").unwrap().online);
    }

    #[test]
    fn force_release_removes_anonymous_session() {
        let reg = AgentRegistry::new();
        reg.try_online(spec("u", "", "s1", 1)).unwrap();

        assert!(reg.force_release("s1", Some(1), 0));
        assert!(reg.get("s1").is_none());
        assert!(reg.list().is_empty());
    }
}
