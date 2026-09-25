use super::model::{
    ApiResponse, ClientInfo, Page, SystemConfig, SystemInfo, SystemStatus, TunnelInfo,
    TunnelTrafficPoint, TunnelTrafficResp,
};
use super::DashState;
use crate::metrics::{TrafficWindow, TunnelTrafficHistory};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use orbien_core::VERSION;
use rust_embed::Embed;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Embed)]
#[folder = "assets/"]
struct Assets;
pub fn router(state: Arc<DashState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/", get(index_html))
        .route("/favicon.ico", get(favicon))
        .route("/api/v1/system/info", get(system_info))
        .route("/api/v1/system/traffic", get(system_traffic))
        .route("/api/v1/clients", get(list_clients))
        .route("/api/v1/clients/{session_id}", get(get_client))
        .route("/api/v1/clients/{session_id}/kick", post(kick_client))
        .route("/api/v1/tunnels", get(list_tunnels))
        .route("/api/v1/tunnels/{name}/traffic", get(tunnel_traffic))
        .route("/{*path}", get(static_file))
        .with_state(state)
}

pub async fn basic_auth(
    State(state): State<Arc<DashState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    if authorized(&state, req.headers()) {
        return Ok(next.run(req).await);
    }
    let mut res = (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    res.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"Restricted\""),
    );
    Err(res)
}

fn authorized(state: &DashState, headers: &HeaderMap) -> bool {
    let Some(h) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(b64) = h
        .strip_prefix("Basic ")
        .or_else(|| h.strip_prefix("basic "))
    else {
        return false;
    };
    let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else {
        return false;
    };
    let Ok(s) = String::from_utf8(raw) else {
        return false;
    };
    let Some((u, p)) = s.split_once(':') else {
        return false;
    };
    u == state.cfg.user && p == state.cfg.password
}

async fn index_html() -> Response {
    serve_asset("index.html")
}

async fn favicon() -> Response {
    try_embedded("favicon.ico").unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

async fn static_file(Path(path): Path<String>) -> Response {
    let rel = path.trim_start_matches('/');
    if !is_safe_asset_path(rel) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(res) = try_embedded(rel) {
        return res;
    }
    serve_asset("index.html")
}

fn serve_asset(path: &str) -> Response {
    if let Some(res) = try_embedded(path) {
        return res;
    }
    (
        StatusCode::NOT_FOUND,
        "dashboard assets missing — run `make web` then rebuild orbien-server",
    )
        .into_response()
}

fn try_embedded(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    Some(bytes_response(content_type(path), file.data.into_owned()))
}

#[derive(Deserialize)]
struct PageQuery {
    #[serde(default = "default_page")]
    page: usize,
    #[serde(default = "default_page_size", rename = "pageSize")]
    page_size: usize,
}

fn default_page() -> usize {
    1
}
fn default_page_size() -> usize {
    50
}

#[derive(Deserialize)]
struct TrafficQuery {
    #[serde(default)]
    range: String,
}

fn traffic_window(q: &TrafficQuery) -> TrafficWindow {
    TrafficWindow::parse(&q.range)
}

fn traffic_resp(hist: TunnelTrafficHistory) -> TunnelTrafficResp {
    TunnelTrafficResp {
        name: hist.name,
        unit: hist.unit,
        granularity: hist.granularity,
        history: hist
            .history
            .into_iter()
            .map(|p| TunnelTrafficPoint {
                date: p.date,
                traffic_in: p.traffic_in,
                traffic_out: p.traffic_out,
            })
            .collect(),
    }
}

async fn system_info(State(state): State<Arc<DashState>>) -> Json<ApiResponse<SystemInfo>> {
    let snap = state.svc.dashboard_snapshot().await;
    Json(ApiResponse::ok(SystemInfo {
        version: VERSION.to_string(),
        config: SystemConfig {
            listen: state.svc.cfg().listen.clone(),
            quic_port: state.svc.cfg().quic_port,
            kcp_port: state.svc.cfg().kcp_port,
            http_gw_port: state.svc.cfg().http_gw_port,
            https_gw_port: state.svc.cfg().https_gw_port,
            root_domain: state.svc.cfg().root_domain.clone(),
            tcp_mux: state.svc.cfg().transport.tcp_mux,
            tls_force: state.svc.cfg().transport.tls.force,
            max_conn_pool: state.svc.cfg().transport.max_conn_pool,
            heartbeat_timeout: state.svc.cfg().transport.heartbeat_timeout,
        },
        status: SystemStatus {
            client_counts: snap
                .clients
                .iter()
                .filter(|c| c.status.is_empty() || c.status == "online")
                .count(),
            total_client_counts: snap.total_client_counts,
            tunnel_type_count: snap.tunnel_type_count,
            active_connections: snap.active_connections,
            total_traffic_in: snap.total_traffic_in,
            total_traffic_out: snap.total_traffic_out,
        },
    }))
}

async fn system_traffic(
    State(state): State<Arc<DashState>>,
    Query(q): Query<TrafficQuery>,
) -> Json<ApiResponse<TunnelTrafficResp>> {
    let hist = state.svc.metrics().server_traffic(traffic_window(&q));
    Json(ApiResponse::ok(traffic_resp(hist)))
}

async fn list_clients(
    State(state): State<Arc<DashState>>,
    Query(q): Query<PageQuery>,
) -> Json<ApiResponse<Page<ClientInfo>>> {
    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 200);
    let snap = state.svc.dashboard_snapshot().await;
    let total = snap.clients.len();
    let start = (page - 1).saturating_mul(page_size);
    Json(ApiResponse::ok(Page {
        total,
        page,
        page_size,
        items: snap
            .clients
            .into_iter()
            .skip(start)
            .take(page_size)
            .collect(),
    }))
}

async fn get_client(
    State(state): State<Arc<DashState>>,
    Path(session_id): Path<String>,
) -> Result<Json<ApiResponse<ClientInfo>>, StatusCode> {
    let session_id = urlencoding_decode(&session_id);
    let snap = state.svc.dashboard_snapshot().await;
    match snap
        .clients
        .into_iter()
        .find(|c| c.session_id == session_id || c.agent_id == session_id)
    {
        Some(c) => Ok(Json(ApiResponse::ok(c))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn kick_client(
    State(state): State<Arc<DashState>>,
    Path(session_id): Path<String>,
) -> Json<ApiResponse<()>> {
    let session_id = urlencoding_decode(&session_id);
    match state.svc.kick_client(&session_id).await {
        Ok(()) => Json(ApiResponse::ok(())),
        Err(e) => Json(ApiResponse {
            code: 404,
            msg: e.to_string(),
            data: (),
        }),
    }
}

#[derive(Deserialize)]
struct TunnelListQuery {
    #[serde(default = "default_page")]
    page: usize,
    #[serde(default = "default_page_size", rename = "pageSize")]
    page_size: usize,
    #[serde(default, rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    q: String,
}

async fn list_tunnels(
    State(state): State<Arc<DashState>>,
    Query(q): Query<TunnelListQuery>,
) -> Json<ApiResponse<Page<TunnelInfo>>> {
    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 200);
    let snap = state.svc.dashboard_snapshot().await;
    let session_id = q.session_id.trim();
    let needle = q.q.trim().to_ascii_lowercase();
    let filtered: Vec<TunnelInfo> = snap
        .tunnels
        .into_iter()
        .filter(|p| session_id.is_empty() || p.session_id == session_id)
        .filter(|p| needle.is_empty() || p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    let total = filtered.len();
    let start = (page - 1).saturating_mul(page_size);
    Json(ApiResponse::ok(Page {
        total,
        page,
        page_size,
        items: filtered.into_iter().skip(start).take(page_size).collect(),
    }))
}

async fn tunnel_traffic(
    State(state): State<Arc<DashState>>,
    Path(name): Path<String>,
    Query(q): Query<TrafficQuery>,
) -> Result<Json<ApiResponse<TunnelTrafficResp>>, StatusCode> {
    let name = urlencoding_decode(&name);
    match state
        .svc
        .metrics()
        .tunnel_traffic(&name, traffic_window(&q))
    {
        Some(hist) => Ok(Json(ApiResponse::ok(traffic_resp(hist)))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

fn urlencoding_decode(raw: &str) -> String {
    percent_decode(raw).unwrap_or_else(|| raw.to_string())
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let h = from_hex(bytes[i + 1])?;
                let l = from_hex(bytes[i + 2])?;
                out.push((h << 4) | l);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn is_safe_asset_path(path: &str) -> bool {
    !path.is_empty() && !path.contains("..")
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".map") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

fn bytes_response(content_type: &'static str, body: Vec<u8>) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], body).into_response()
}
