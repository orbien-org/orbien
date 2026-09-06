use crate::control::Control;
use crate::dashboard::model::{ClientInfo, TunnelInfo};
use crate::service::Service;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct DashboardSnapshot {
    pub clients: Vec<ClientInfo>,
    pub tunnels: Vec<TunnelInfo>,
    pub tunnel_type_count: BTreeMap<String, usize>,
    pub active_connections: usize,
    pub total_client_counts: usize,
    pub total_traffic_in: u64,
    pub total_traffic_out: u64,
}

impl Service {
    pub async fn dashboard_snapshot(&self) -> DashboardSnapshot {
        let online: Vec<Arc<Control>> = {
            let controls = self.controls.lock().await;
            controls
                .values()
                .map(|entry| Arc::clone(&entry.control))
                .collect()
        };
        let agents = self.agents.list();

        let mut clients = Vec::with_capacity(online.len() + agents.len());
        let mut tunnels = Vec::new();
        let mut tunnel_type_count: BTreeMap<String, usize> = BTreeMap::new();
        let mut online_ids = std::collections::HashSet::new();

        for ctrl in &online {
            let tunnel_count = ctrl.tunnel_count().await;
            online_ids.insert(ctrl.session_id.clone());
            let mut active_connections = 0usize;
            let mut client_tunnels = Vec::new();
            for s in ctrl.tunnel_summaries().await {
                *tunnel_type_count.entry(s.tunnel_type.clone()).or_default() += 1;
                let traffic = self.metrics.tunnel_snapshot(&s.name);
                let tunnel_conns = traffic.as_ref().map(|t| t.active_connections).unwrap_or(0);
                active_connections += tunnel_conns;
                client_tunnels.push(TunnelInfo {
                    name: s.name,
                    tunnel_type: s.tunnel_type,
                    remote_addr: s.remote_addr,
                    local_addr: s.local_addr,
                    session_id: ctrl.session_id.clone(),
                    status: s.status,
                    today_traffic_in: traffic.as_ref().map(|t| t.today_traffic_in).unwrap_or(0),
                    today_traffic_out: traffic.as_ref().map(|t| t.today_traffic_out).unwrap_or(0),
                    active_connections: tunnel_conns,
                    last_start_time: traffic
                        .as_ref()
                        .and_then(|t| format_tunnel_time(t.last_start_at)),
                });
            }
            clients.push(ClientInfo {
                session_id: ctrl.session_id.clone(),
                agent_id: ctrl.agent_id.clone(),
                user: ctrl.user.clone(),
                hostname: ctrl.hostname.clone(),
                os: ctrl.os.clone(),
                arch: ctrl.arch.clone(),
                client_ip: ctrl.client_ip.clone(),
                version: ctrl.version.clone(),
                tunnel_count,
                active_connections,
                connected_secs: ctrl.connected_at.elapsed().as_secs(),
                status: "online".into(),
            });
            tunnels.extend(client_tunnels);
        }

        for rec in agents {
            if rec.online || online_ids.contains(&rec.session_id) {
                continue;
            }
            clients.push(ClientInfo {
                session_id: rec.session_id.clone(),
                agent_id: rec.agent_id.clone(),
                user: rec.user.clone(),
                hostname: rec.hostname.clone(),
                os: rec.os.clone(),
                arch: rec.arch.clone(),
                client_ip: rec.client_ip.clone(),
                version: rec.version.clone(),
                tunnel_count: rec.tunnel_count,
                active_connections: 0,
                connected_secs: rec
                    .disconnected_at
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0),
                status: "offline".into(),
            });
        }

        clients.sort_by(|a, b| {
            let ao = a.status == "online";
            let bo = b.status == "online";
            bo.cmp(&ao)
                .then_with(|| a.display_id().cmp(b.display_id()))
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        tunnels.sort_by(|a, b| a.name.cmp(&b.name).then(a.session_id.cmp(&b.session_id)));

        let server_stats = self.metrics.server_snapshot();
        let total_clients = clients.len();

        DashboardSnapshot {
            clients,
            tunnels,
            tunnel_type_count,
            active_connections: server_stats.active_connections,
            total_client_counts: total_clients,
            total_traffic_in: server_stats.total_traffic_in,
            total_traffic_out: server_stats.total_traffic_out,
        }
    }
}

fn format_tunnel_time(unix: Option<i64>) -> Option<String> {
    let ts = unix?;
    let dt = chrono::DateTime::from_timestamp(ts, 0)?;
    Some(
        dt.with_timezone(&chrono::Local)
            .format("%m-%d %H:%M:%S")
            .to_string(),
    )
}
