#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod client_log_layer;
mod config_bridge;
mod i18n;
mod log_buffer;
#[cfg(target_os = "macos")]
mod macos_reopen;
mod pick_file;
mod process_stats;
mod runtime;
mod ui_prefs;

use i18n::Locale;
use log_buffer::{LogStore, UiSyncCursor};
use process_stats::ProcessMeter;
use slint::Model;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ui_prefs::UiPrefs;

slint::include_modules!();

fn locale_of(ui: &AppWindow) -> Locale {
    Locale::from_index(ui.get_locale_index())
}

fn sync_tray_locale(tray: &OrbienTray, locale_index: i32) {
    let tr = tray.global::<Tr>();
    if tr.get_locale_index() != locale_index {
        tr.set_locale_index(locale_index);
    }
}

fn reveal_main_window(ui: &AppWindow) {
    let _ = ui.show();
    use slint::winit_030::WinitWindowAccessor;
    let window = ui.window();
    if window.has_winit_window() {
        window.with_winit_window(|w| {
            w.focus_window();
        });
    }
}

fn apply_theme(ui: &AppWindow, theme_index: i32) {
    ui.set_theme_index(theme_index);
}

fn sync_about_theme(about: &AboutDialog, theme_index: i32) {
    about.invoke_apply_theme(theme_index);
}

fn persist_prefs(ui: &AppWindow) {
    let mut prefs = UiPrefs::default();
    prefs.set_locale_index(ui.get_locale_index());
    prefs.set_theme_index(ui.get_theme_index());
    if let Err(e) = ui_prefs::save(&prefs) {
        tracing::warn!(?e, "failed to save ui prefs");
    }
}

#[derive(Clone, Copy)]
enum ToastKind {
    Success = 0,
    Error = 1,
}

fn show_toast(ui: &AppWindow, text: impl Into<slint::SharedString>, kind: ToastKind) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    ui.set_toast_text(text);
    ui.set_toast_kind(kind as i32);
    ui.set_toast_visible(true);
    ui.set_toast_token(ui.get_toast_token().wrapping_add(1));
}

fn toast_ok(ui: &AppWindow, text: impl Into<slint::SharedString>) {
    show_toast(ui, text, ToastKind::Success);
}

fn toast_err(ui: &AppWindow, text: impl Into<slint::SharedString>) {
    show_toast(ui, text, ToastKind::Error);
}

fn require_port_field(
    raw: &str,
    required: impl Into<slint::SharedString>,
    invalid: impl Into<slint::SharedString>,
) -> Result<(), slint::SharedString> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(required.into());
    }
    if t.parse::<u16>().is_err() {
        return Err(invalid.into());
    }
    Ok(())
}

fn row_to_tunnel(row: &TunnelRow) -> anyhow::Result<orbien_core::config::TunnelConfig> {
    config_bridge::tunnel_from_parts(
        row.name.as_str(),
        row.tunnel_type.as_str(),
        row.local_ip.as_str(),
        row.local_port.as_str(),
        row.remote_port.as_str(),
        row.domains.as_str(),
        row.locations.as_str(),
        row.basic_auth_user.as_str(),
        row.basic_auth_password.as_str(),
        row.host_header_rewrite.as_str(),
        row.bandwidth_limit.as_str(),
        row.bandwidth_limit_side.as_str(),
        row.proxy_protocol_version.as_str(),
        row.plugin_tls_term,
        row.plugin_local_addr.as_str(),
        row.plugin_cert_file.as_str(),
        row.plugin_key_file.as_str(),
        row.plugin_host_rewrite.as_str(),
        row.plugin_username.as_str(),
        row.plugin_password.as_str(),
    )
}

fn tunnel_to_row(p: &orbien_core::config::TunnelConfig) -> TunnelRow {
    let parts = config_bridge::tunnel_to_parts(p);
    TunnelRow {
        name: parts.name.into(),
        tunnel_type: parts.tunnel_type.into(),
        local_ip: parts.local_ip.into(),
        local_port: parts.local_port.into(),
        remote_port: parts.remote_port.into(),
        remote_addr: "".into(),
        domains: parts.domains.into(),
        locations: parts.locations.into(),
        basic_auth_user: parts.basic_auth_user.into(),
        basic_auth_password: parts.basic_auth_password.into(),
        host_header_rewrite: parts.host_header_rewrite.into(),
        bandwidth_limit: parts.bandwidth.into(),
        bandwidth_limit_side: parts.bandwidth_limit_side.into(),
        proxy_protocol_version: parts.proxy_protocol_version.into(),
        plugin_tls_term: parts.plugin_tls_term,
        plugin_local_addr: parts.plugin_local_addr.into(),
        plugin_cert_file: parts.plugin_cert_file.into(),
        plugin_key_file: parts.plugin_key_file.into(),
        plugin_host_rewrite: parts.plugin_host_rewrite.into(),
        plugin_username: parts.plugin_username.into(),
        plugin_password: parts.plugin_password.into(),
    }
}

fn collect_tunnel_configs(
    rows: &[TunnelRow],
) -> anyhow::Result<Vec<orbien_core::config::TunnelConfig>> {
    rows.iter()
        .filter(|r| !r.name.trim().is_empty())
        .map(row_to_tunnel)
        .collect()
}

fn apply_config_to_ui(ui: &AppWindow, cfg: &orbien_client::ClientConfig) {
    let (server_host, server_port) = config_bridge::split_server_endpoint(&cfg.server);
    ui.set_server_addr(server_host.into());
    ui.set_server_port(server_port.into());
    ui.set_token(cfg.auth.token.clone().into());
    ui.set_user(cfg.user.clone().into());
    let protocol_idx = config_bridge::protocol_index(&cfg.transport.protocol);
    let is_quic = protocol_idx == 2;
    ui.set_protocol_index(protocol_idx);
    ui.set_pool_count(cfg.transport.pool_count.to_string().into());
    ui.set_ws_path(config_bridge::ws_path_display(&cfg.transport.ws_path).into());
    ui.set_tcp_mux(!is_quic && cfg.transport.tcp_mux);
    ui.set_tls_enable(is_quic || cfg.transport.tls.enable);
    ui.set_config_mux_keepalive(cfg.transport.mux_keepalive_secs.to_string().into());
    ui.set_config_heartbeat_interval(
        config_bridge::optional_i64_display(cfg.transport.heartbeat_interval).into(),
    );
    ui.set_config_heartbeat_timeout(
        config_bridge::optional_i64_display(cfg.transport.heartbeat_timeout).into(),
    );
    ui.set_config_udp_packet_size(cfg.udp_packet_size.to_string().into());
    ui.set_config_tls_server_name(cfg.transport.tls.server_name.clone().into());
    ui.set_config_tls_ca(cfg.transport.tls.trusted_ca_file.clone().into());
    ui.set_config_tls_cert(cfg.transport.tls.cert_file.clone().into());
    ui.set_config_tls_key(cfg.transport.tls.key_file.clone().into());
    ui.set_config_quic_keepalive(cfg.transport.quic.keepalive_period.to_string().into());
    ui.set_config_quic_idle(cfg.transport.quic.max_idle_timeout.to_string().into());
    ui.set_config_quic_streams(cfg.transport.quic.max_incoming_streams.to_string().into());
    let rows: Vec<TunnelRow> = cfg.tunnels.iter().map(tunnel_to_row).collect();
    ui.set_tunnels(slint::ModelRc::new(slint::VecModel::from(rows)));
}

fn persist_tunnels(ui: &AppWindow, rows: &[TunnelRow]) -> Result<(), String> {
    let tunnels = collect_tunnel_configs(rows).map_err(|e| e.to_string())?;
    let (cfg, path) = config_bridge::load_merge_tunnels(
        &ui.get_config_file_path(),
        &ui.get_server_addr(),
        &ui.get_server_port(),
        &ui.get_token(),
        &ui.get_user(),
        ui.get_protocol_index(),
        &ui.get_pool_count(),
        ui.get_tcp_mux(),
        ui.get_tls_enable(),
        tunnels,
    )
    .map_err(|e| e.to_string())?;
    config_bridge::save(&path, &cfg).map_err(|e| e.to_string())?;
    ui.set_config_file_path(config_bridge::path_display(&path).into());
    apply_if_running_async(ui, cfg, path);
    Ok(())
}

fn persist_server_config(ui: &AppWindow) -> Result<(), String> {
    let model = ui.get_tunnels();
    let rows: Vec<TunnelRow> = (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .collect();
    let tunnels = collect_tunnel_configs(&rows).map_err(|e| e.to_string())?;
    let (cfg, path) = config_bridge::load_merge_server_fields(
        &ui.get_config_file_path(),
        &ui.get_server_addr(),
        &ui.get_server_port(),
        &ui.get_token(),
        &ui.get_user(),
        ui.get_protocol_index(),
        &ui.get_pool_count(),
        ui.get_tcp_mux(),
        ui.get_tls_enable(),
        &ui.get_ws_path(),
        &ui.get_config_mux_keepalive(),
        &ui.get_config_heartbeat_interval(),
        &ui.get_config_heartbeat_timeout(),
        &ui.get_config_udp_packet_size(),
        &ui.get_config_tls_server_name(),
        &ui.get_config_tls_ca(),
        &ui.get_config_tls_cert(),
        &ui.get_config_tls_key(),
        &ui.get_config_quic_keepalive(),
        &ui.get_config_quic_idle(),
        &ui.get_config_quic_streams(),
        tunnels,
    )
    .map_err(|e| e.to_string())?;
    config_bridge::save(&path, &cfg).map_err(|e| e.to_string())?;
    ui.set_config_file_path(config_bridge::path_display(&path).into());
    apply_if_running_async(ui, cfg, path);
    Ok(())
}

fn apply_if_running_async(
    ui: &AppWindow,
    cfg: orbien_client::ClientConfig,
    path: std::path::PathBuf,
) {
    if !runtime::status().is_active() {
        return;
    }
    push_log(ui, "INFO  applying reload");
    let ui_weak = ui.as_weak();
    runtime::reload_async(cfg, path, move |result| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let loc = locale_of(&ui);
        match result {
            Ok(outcome) => finish_reload_ui(&ui, loc, &outcome),
            Err(e) => {
                toast_err(&ui, i18n::reload_failed(loc, &e.to_string()));
                push_log(&ui, &format!("ERROR reload failed: {e}"));
            }
        }
    });
}

fn finish_reload_ui(ui: &AppWindow, loc: Locale, outcome: &orbien_client::ReloadOutcome) {
    push_log(
        ui,
        &format!(
            "INFO  reload applied: +{} -{} ~{} mode={}",
            outcome.added.len(),
            outcome.removed.len(),
            outcome.updated.len(),
            outcome.level.label()
        ),
    );
    for (name, err) in &outcome.failed {
        push_log(ui, &format!("WARN  reload tunnel {name}: {err}"));
    }
    if !outcome.succeeded() {
        toast_err(&ui, i18n::reload_partial(loc));
        return;
    }
    match outcome.level {
        orbien_client::ReloadLevel::ReconnectControl => {
            toast_ok(&ui, i18n::config_saved_reconnecting(loc));
        }
        orbien_client::ReloadLevel::TunnelsOnly => {
            toast_ok(&ui, i18n::config_saved_applied(loc));
        }
        _ => {
            toast_ok(&ui, i18n::config_saved(loc));
        }
    }
}

fn type_index(tunnel_type: &str) -> i32 {
    match tunnel_type {
        "udp" => 1,
        "http" => 2,
        "https" => 3,
        "socks5" => 4,
        _ => 0,
    }
}

fn type_name(index: i32) -> &'static str {
    match index {
        1 => "udp",
        2 => "http",
        3 => "https",
        4 => "socks5",
        _ => "tcp",
    }
}

fn bandwidth_side_index(mode: &str) -> i32 {
    if mode == "server" {
        1
    } else {
        0
    }
}

fn bandwidth_side_name(index: i32) -> &'static str {
    if index == 1 {
        "server"
    } else {
        "client"
    }
}

fn proxy_protocol_index(version: &str) -> i32 {
    match version {
        "v1" => 1,
        "v2" => 2,
        _ => 0,
    }
}

fn proxy_protocol_name(index: i32) -> &'static str {
    match index {
        1 => "v1",
        2 => "v2",
        _ => "",
    }
}

fn reset_tunnel_form(ui: &AppWindow) {
    ui.set_tunnel_edit_name("".into());
    ui.set_tunnel_edit_local_ip("127.0.0.1".into());
    ui.set_tunnel_edit_local_port("8080".into());
    ui.set_tunnel_edit_remote_port("9000".into());
    ui.set_tunnel_edit_domains("".into());
    ui.set_tunnel_edit_locations("".into());
    ui.set_tunnel_edit_basic_auth_user("".into());
    ui.set_tunnel_edit_basic_auth_password("".into());
    ui.set_tunnel_edit_host_header_rewrite("".into());
    ui.set_tunnel_edit_bandwidth_limit("".into());
    ui.set_tunnel_edit_bandwidth_side_index(0);
    ui.set_tunnel_edit_proxy_protocol_index(0);
    ui.set_tunnel_edit_plugin_tls_term(false);
    ui.set_tunnel_edit_plugin_local_addr("127.0.0.1:80".into());
    ui.set_tunnel_edit_plugin_cert_file("".into());
    ui.set_tunnel_edit_plugin_key_file("".into());
    ui.set_tunnel_edit_plugin_host_rewrite("".into());
    ui.set_tunnel_edit_plugin_username("".into());
    ui.set_tunnel_edit_plugin_password("".into());
    ui.set_tunnel_edit_type_index(0);
    ui.set_tunnel_show_advanced(false);
}

fn fill_tunnel_form(ui: &AppWindow, row: &TunnelRow) {
    ui.set_tunnel_edit_name(row.name.clone());
    ui.set_tunnel_edit_type_index(type_index(row.tunnel_type.as_str()));
    ui.set_tunnel_edit_local_ip(row.local_ip.clone());
    ui.set_tunnel_edit_local_port(row.local_port.clone());
    ui.set_tunnel_edit_remote_port(row.remote_port.clone());
    ui.set_tunnel_edit_domains(row.domains.clone());
    ui.set_tunnel_edit_locations(row.locations.clone());
    ui.set_tunnel_edit_basic_auth_user(row.basic_auth_user.clone());
    ui.set_tunnel_edit_basic_auth_password(row.basic_auth_password.clone());
    ui.set_tunnel_edit_host_header_rewrite(row.host_header_rewrite.clone());
    ui.set_tunnel_edit_bandwidth_limit(row.bandwidth_limit.clone());
    ui.set_tunnel_edit_bandwidth_side_index(bandwidth_side_index(
        row.bandwidth_limit_side.as_str(),
    ));
    ui.set_tunnel_edit_proxy_protocol_index(proxy_protocol_index(
        row.proxy_protocol_version.as_str(),
    ));
    ui.set_tunnel_edit_plugin_tls_term(row.plugin_tls_term);
    ui.set_tunnel_edit_plugin_local_addr(row.plugin_local_addr.clone());
    ui.set_tunnel_edit_plugin_cert_file(row.plugin_cert_file.clone());
    ui.set_tunnel_edit_plugin_key_file(row.plugin_key_file.clone());
    ui.set_tunnel_edit_plugin_host_rewrite(row.plugin_host_rewrite.clone());
    ui.set_tunnel_edit_plugin_username(row.plugin_username.clone());
    ui.set_tunnel_edit_plugin_password(row.plugin_password.clone());
    ui.set_tunnel_show_advanced(false);
}

fn collect_tunnel_form(ui: &AppWindow) -> TunnelRow {
    let ty = type_name(ui.get_tunnel_edit_type_index());
    let is_socks5 = ty == "socks5";
    let is_http = ty == "http";
    let is_https = ty == "https";
    let plugin = is_https && ui.get_tunnel_edit_plugin_tls_term();
    TunnelRow {
        name: ui.get_tunnel_edit_name(),
        tunnel_type: ty.into(),
        local_ip: if plugin || is_socks5 {
            "".into()
        } else {
            ui.get_tunnel_edit_local_ip()
        },
        local_port: if plugin || is_socks5 {
            "".into()
        } else {
            ui.get_tunnel_edit_local_port()
        },
        remote_port: if is_http || is_https {
            "0".into()
        } else {
            ui.get_tunnel_edit_remote_port()
        },
        remote_addr: "".into(),
        domains: if is_http || is_https {
            ui.get_tunnel_edit_domains()
        } else {
            "".into()
        },
        locations: if is_http {
            ui.get_tunnel_edit_locations()
        } else {
            "".into()
        },
        basic_auth_user: if is_http {
            ui.get_tunnel_edit_basic_auth_user()
        } else {
            "".into()
        },
        basic_auth_password: if is_http {
            ui.get_tunnel_edit_basic_auth_password()
        } else {
            "".into()
        },
        host_header_rewrite: if is_http {
            ui.get_tunnel_edit_host_header_rewrite()
        } else {
            "".into()
        },
        bandwidth_limit: ui.get_tunnel_edit_bandwidth_limit(),
        bandwidth_limit_side: bandwidth_side_name(ui.get_tunnel_edit_bandwidth_side_index()).into(),
        proxy_protocol_version: if plugin {
            "".into()
        } else {
            proxy_protocol_name(ui.get_tunnel_edit_proxy_protocol_index()).into()
        },
        plugin_tls_term: plugin,
        plugin_local_addr: if plugin {
            ui.get_tunnel_edit_plugin_local_addr()
        } else {
            "".into()
        },
        plugin_cert_file: if plugin {
            ui.get_tunnel_edit_plugin_cert_file()
        } else {
            "".into()
        },
        plugin_key_file: if plugin {
            ui.get_tunnel_edit_plugin_key_file()
        } else {
            "".into()
        },
        plugin_host_rewrite: if plugin {
            ui.get_tunnel_edit_plugin_host_rewrite()
        } else {
            "".into()
        },
        plugin_username: if is_socks5 {
            ui.get_tunnel_edit_plugin_username()
        } else {
            "".into()
        },
        plugin_password: if is_socks5 {
            ui.get_tunnel_edit_plugin_password()
        } else {
            "".into()
        },
    }
}

fn omit_gateway_port(tunnel_type: &str) -> Option<u16> {
    let t = tunnel_type.trim();
    if t.eq_ignore_ascii_case("http") {
        Some(80)
    } else if t.eq_ignore_ascii_case("https") {
        Some(443)
    } else {
        None
    }
}

fn split_host_port(s: &str) -> Option<(&str, &str)> {
    if s.starts_with('[') {
        let close = s.find(']')?;
        let host = &s[..=close];
        let port = s.get(close + 1..)?.strip_prefix(':')?;
        if port.is_empty() {
            return None;
        }
        return Some((host, port));
    }
    let (host, port) = s.rsplit_once(':')?;
    if host.is_empty() || port.is_empty() || host.contains(':') {
        None
    } else {
        Some((host, port))
    }
}

fn strip_omitted_port(hostport: &str, omit: u16) -> &str {
    match split_host_port(hostport) {
        Some((host, port)) if port.parse() == Ok(omit) => host,
        _ => hostport,
    }
}

fn display_remote_addr(tunnel_type: &str, addr: &str) -> String {
    let Some(omit) = omit_gateway_port(tunnel_type) else {
        return addr.to_string();
    };
    let addr = addr.trim();
    if addr.is_empty() {
        return String::new();
    }
    if !addr.contains(',') {
        return strip_omitted_port(addr, omit).to_string();
    }
    let mut out = String::with_capacity(addr.len());
    for part in addr.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(strip_omitted_port(part, omit));
    }
    out
}

fn apply_tunnel_remotes(ui: &AppWindow, remotes: &std::collections::HashMap<String, String>) {
    let model = ui.get_tunnels();
    let Some(vm) = model.as_any().downcast_ref::<slint::VecModel<TunnelRow>>() else {
        return;
    };
    let n = vm.row_count();
    for i in 0..n {
        let Some(mut row) = vm.row_data(i) else {
            continue;
        };
        let raw = remotes
            .get(row.name.as_str())
            .map(String::as_str)
            .unwrap_or("");
        let next = display_remote_addr(row.tunnel_type.as_str(), raw);
        if row.remote_addr.as_str() == next {
            continue;
        }
        row.remote_addr = next.into();
        vm.set_row_data(i, row);
    }
}

fn retain_remote_addrs(rows: &mut [TunnelRow], previous: &[TunnelRow]) {
    for row in rows.iter_mut() {
        if !row.remote_addr.is_empty() {
            continue;
        }
        let Some(prev) = previous.iter().find(|p| p.name == row.name) else {
            continue;
        };
        if prev.domains != row.domains
            || prev.remote_port != row.remote_port
            || prev.tunnel_type != row.tunnel_type
        {
            continue;
        }
        if !prev.remote_addr.is_empty() {
            row.remote_addr = prev.remote_addr.clone();
        }
    }
}

fn copy_to_clipboard(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let Ok(mut child) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() else {
            return false;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            if stdin.write_all(text.as_bytes()).is_err() {
                return false;
            }
        }
        return child.wait().map(|s| s.success()).unwrap_or(false);
    }
    #[cfg(target_os = "windows")]
    {
        let Ok(mut child) = Command::new("cmd")
            .args(["/C", "clip"])
            .stdin(Stdio::piped())
            .spawn()
        else {
            return false;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            if stdin.write_all(text.as_bytes()).is_err() {
                return false;
            }
        }
        return child.wait().map(|s| s.success()).unwrap_or(false);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for cmd in [
            ("wl-copy", vec![] as Vec<&str>),
            ("xclip", vec!["-selection", "clipboard"]),
            ("xsel", vec!["--clipboard", "--input"]),
        ] {
            let Ok(mut child) = Command::new(cmd.0)
                .args(&cmd.1)
                .stdin(Stdio::piped())
                .spawn()
            else {
                continue;
            };
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(text.as_bytes()).is_err() {
                    continue;
                }
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
        false
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = text;
        false
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let _ = slint::BackendSelector::new()
        .backend_name("winit".into())
        .select();

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().expect("directive")),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(client_log_layer::ClientUiLogLayer::new(runtime::handle()))
        .init();

    let ui = AppWindow::new()?;
    let prefs = ui_prefs::load();
    ui.set_locale_index(prefs.locale_index());
    let _ = ui.global::<Tr>().set_locale_index(ui.get_locale_index());
    apply_theme(&ui, prefs.theme_index());

    let tray = OrbienTray::new()?;
    sync_tray_locale(&tray, ui.get_locale_index());
    let tray_weak = tray.as_weak();
    {
        let ui_weak = ui.as_weak();
        tray.on_show_window(move || {
            if let Some(ui) = ui_weak.upgrade() {
                reveal_main_window(&ui);
            }
        });
    }
    #[cfg(target_os = "macos")]
    {
        let ui_weak = ui.as_weak();
        macos_reopen::install(move || {
            if let Some(ui) = ui_weak.upgrade() {
                reveal_main_window(&ui);
            }
        });
    }
    tray.on_quit(|| {
        runtime::stop_async(|| {
            let _ = slint::quit_event_loop();
        });
    });

    let default_path = config_bridge::default_config_path();
    ui.set_config_file_path(config_bridge::path_display(&default_path).into());

    ui.set_log_lines(slint::ModelRc::from(log_buffer::make_model()));
    log_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_raw(&format!(
            "INFO  Orbien Desktop {} ready",
            orbien_client::VERSION
        ));

    if default_path.is_file() {
        match orbien_client::ClientConfig::load_for_edit(&default_path) {
            Ok(mut cfg) => {
                if config_bridge::ensure_agent_id(&mut cfg) {
                    if let Err(e) = config_bridge::save(&default_path, &cfg) {
                        log_store()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push_raw(&format!("WARN  failed to persist agent_id: {e}"));
                    } else {
                        log_store()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push_raw(&format!("INFO  assigned agent_id {}", cfg.agent_id));
                    }
                }
                apply_config_to_ui(&ui, &cfg);
                log_store()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_raw(&format!("INFO  loaded config {}", default_path.display()));
            }
            Err(e) => {
                ui.set_tunnels(slint::ModelRc::new(slint::VecModel::<TunnelRow>::from(
                    Vec::new(),
                )));
                log_store()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_raw(&format!("ERROR failed to load config: {e}"));
            }
        }
    } else {
        ui.set_tunnels(slint::ModelRc::new(slint::VecModel::<TunnelRow>::from(
            Vec::new(),
        )));
        log_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_raw("INFO  no config file yet; Config page fields used on first start/save");
    }

    let started_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

    let ui_weak = ui.as_weak();
    ui.on_toggle_client({
        let started_at = started_at.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            if ui.get_busy() {
                return;
            }
            let loc = locale_of(&ui);
            ui.set_busy(true);

            if runtime::status().is_active() {
                runtime::stop_async(|| {});
                *started_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                ui.set_running(false);
                ui.set_running_label("—".into());
                push_log(&ui, &i18n::client_stopped(loc));
            } else {
                let model = ui.get_tunnels();
                let rows: Vec<TunnelRow> = (0..model.row_count())
                    .filter_map(|i| model.row_data(i))
                    .collect();
                let start_cfg =
                    (|| -> Result<(orbien_client::ClientConfig, std::path::PathBuf), String> {
                        let tunnels = collect_tunnel_configs(&rows).map_err(|e| e.to_string())?;
                        config_bridge::load_merge_server_fields(
                            &ui.get_config_file_path(),
                            &ui.get_server_addr(),
                            &ui.get_server_port(),
                            &ui.get_token(),
                            &ui.get_user(),
                            ui.get_protocol_index(),
                            &ui.get_pool_count(),
                            ui.get_tcp_mux(),
                            ui.get_tls_enable(),
                            &ui.get_ws_path(),
                            &ui.get_config_mux_keepalive(),
                            &ui.get_config_heartbeat_interval(),
                            &ui.get_config_heartbeat_timeout(),
                            &ui.get_config_udp_packet_size(),
                            &ui.get_config_tls_server_name(),
                            &ui.get_config_tls_ca(),
                            &ui.get_config_tls_cert(),
                            &ui.get_config_tls_key(),
                            &ui.get_config_quic_keepalive(),
                            &ui.get_config_quic_idle(),
                            &ui.get_config_quic_streams(),
                            tunnels,
                        )
                        .map_err(|e| e.to_string())
                    })();
                match start_cfg {
                    Ok((mut cfg, path)) => {
                        if config_bridge::ensure_agent_id(&mut cfg) {
                            if let Err(e) = config_bridge::save(&path, &cfg) {
                                ui.set_running(false);
                                let detail = e.to_string();
                                toast_err(&ui, i18n::client_start_failed(loc, &detail));
                                push_log(
                                    &ui,
                                    &format!("ERROR failed to persist agent_id: {detail}"),
                                );
                                ui.set_busy(false);
                                return;
                            }
                            push_log(&ui, &format!("INFO  assigned agent_id {}", cfg.agent_id));
                        }
                        tracing::info!(
                            config = %path.display(),
                            server = %cfg.server_endpoint(),
                            agent_id = %cfg.agent_id,
                            "starting in-process client"
                        );
                        match runtime::start(cfg, path) {
                            Ok(()) => {
                                *started_at.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(Instant::now());
                                ui.set_running(true);
                                ui.set_running_label(i18n::running_label_zero(loc).into());
                                push_log(&ui, &i18n::client_started(loc));
                            }
                            Err(e) => {
                                ui.set_running(false);
                                let detail = e.to_string();
                                toast_err(&ui, i18n::client_start_failed(loc, &detail));
                                push_log(&ui, &format!("ERROR failed to start: {detail}"));
                            }
                        }
                    }
                    Err(e) => {
                        ui.set_running(false);
                        let detail = e.to_string();
                        toast_err(&ui, i18n::client_start_failed(loc, &detail));
                        push_log(&ui, &format!("ERROR failed to start: {detail}"));
                    }
                }
            }
            ui.set_busy(false);
        }
    });

    let ui_weak = ui.as_weak();
    let started_at_tick = started_at.clone();
    let remotes_gen = Arc::new(Mutex::new(0u64));
    let last_uptime_secs = Arc::new(Mutex::new(u64::MAX));
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), {
        let remotes_gen = remotes_gen.clone();
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            let drained = runtime::drain_logs();
            if !drained.is_empty() {
                push_logs(&ui, &drained);
            }

            {
                let mut gen = remotes_gen.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((next_gen, map)) = runtime::tunnel_remotes_if_changed(*gen) {
                    *gen = next_gen;
                    apply_tunnel_remotes(&ui, &map);
                }
            }

            let st = runtime::status();
            let running = st.is_active();
            let was_running = ui.get_running();
            if was_running != running {
                ui.set_running(running);
                if was_running && !running {
                    *started_at_tick.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    *last_uptime_secs.lock().unwrap_or_else(|e| e.into_inner()) = u64::MAX;
                    ui.set_running_label("—".into());
                    if let Some(err) = runtime::take_last_error() {
                        let loc = locale_of(&ui);
                        let detail = err.trim();
                        if !detail.is_empty() {
                            toast_err(&ui, i18n::client_stopped_error(loc, detail));
                            push_log(&ui, &format!("ERROR {detail}"));
                        }
                    }
                }
            }
            if running {
                let loc = locale_of(&ui);
                let secs = started_at_tick
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                let mut last = last_uptime_secs.lock().unwrap_or_else(|e| e.into_inner());
                if *last != secs {
                    *last = secs;
                    ui.set_running_label(i18n::format_uptime(loc, secs).into());
                }
            }
        }
    });
    std::mem::forget(timer);

    let ui_weak = ui.as_weak();
    let meter = Arc::new(Mutex::new(ProcessMeter::new()));
    {
        let mut m = meter.lock().unwrap_or_else(|e| e.into_inner());
        let s = m.sample();
        ui.set_sidebar_cpu(process_stats::format_cpu(s.cpu_percent).into());
        ui.set_sidebar_mem(process_stats::format_memory(s.memory_bytes).into());
    }
    let last_sidebar_cpu = Arc::new(Mutex::new(String::new()));
    let last_sidebar_mem = Arc::new(Mutex::new(String::new()));
    let stats_timer = slint::Timer::default();
    stats_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(2),
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            let sample = meter.lock().unwrap_or_else(|e| e.into_inner()).sample();
            let cpu = process_stats::format_cpu(sample.cpu_percent);
            let mem = process_stats::format_memory(sample.memory_bytes);
            {
                let mut prev = last_sidebar_cpu.lock().unwrap_or_else(|e| e.into_inner());
                if *prev != cpu {
                    *prev = cpu.clone();
                    ui.set_sidebar_cpu(cpu.into());
                }
            }
            {
                let mut prev = last_sidebar_mem.lock().unwrap_or_else(|e| e.into_inner());
                if *prev != mem {
                    *prev = mem.clone();
                    ui.set_sidebar_mem(mem.into());
                }
            }
        },
    );
    std::mem::forget(stats_timer);

    let ui_weak = ui.as_weak();
    ui.on_open_logger(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_page(AppPage::Logger);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_page_changed(move |page| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if page == AppPage::Logger {
            flush_logs_to_ui(&ui);
        } else {
            reset_log_ui_cursor();
            ui.set_log_lines(slint::ModelRc::from(log_buffer::make_model()));
        }
    });

    wire_tunnel_and_config(&ui, remotes_gen.clone(), tray_weak);

    let ui_weak = ui.as_weak();
    ui.on_show_about(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        ui.global::<Tr>().set_locale_index(ui.get_locale_index());
        match AboutDialog::new() {
            Ok(about) => {
                about.global::<Tr>().set_locale_index(ui.get_locale_index());
                sync_about_theme(&about, ui.get_theme_index());
                let about_weak = about.as_weak();
                about.on_close_clicked(move || {
                    if let Some(dlg) = about_weak.upgrade() {
                        let _ = dlg.hide();
                    }
                });
                if let Err(err) = about.show() {
                    tracing::warn!(?err, "failed to show About dialog");
                }
            }
            Err(err) => tracing::warn!(?err, "failed to create About dialog"),
        }
    });

    ui.on_open_url(move |url| {
        if let Err(err) = open_url(url.as_ref()) {
            tracing::warn!(%url, ?err, "failed to open url");
        }
    });

    ui.show()?;
    center_window_on_screen(&ui);
    let result = ui.run();
    drop(tray);
    result
}

fn center_window_on_screen(ui: &AppWindow) {
    use slint::winit_030::{winit, WinitWindowAccessor};
    use slint::ComponentHandle;

    let window = ui.window();
    if !window.has_winit_window() {
        return;
    }
    window.with_winit_window(|winit_window| {
        let Some(monitor) = winit_window
            .current_monitor()
            .or_else(|| winit_window.primary_monitor())
        else {
            return;
        };
        let screen = monitor.size();
        let monitor_pos = monitor.position();
        let outer = winit_window.outer_size();
        if outer.width == 0 || outer.height == 0 || screen.width == 0 || screen.height == 0 {
            return;
        }
        let x = monitor_pos.x + ((screen.width as i32 - outer.width as i32) / 2).max(0);
        let y = monitor_pos.y + ((screen.height as i32 - outer.height as i32) / 2).max(0);
        winit_window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
    });
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}

fn wire_tunnel_and_config(
    ui: &AppWindow,
    remotes_gen: Arc<Mutex<u64>>,
    tray_weak: slint::Weak<OrbienTray>,
) {
    let ui_weak = ui.as_weak();
    ui.on_tunnel_add(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_tunnel_editing_index(-1);
            reset_tunnel_form(&ui);
            ui.set_tunnel_editor_open(true);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_tunnel_cancel(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_tunnel_editor_open(false);
            ui.set_tunnel_editing_index(-1);
        }
    });

    let ui_weak = ui.as_weak();
    let remotes_gen_save = remotes_gen.clone();
    ui.on_tunnel_save(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let loc = locale_of(&ui);
        let name = ui.get_tunnel_edit_name().to_string();
        if name.trim().is_empty() {
            toast_err(&ui, i18n::tunnel_name_required(loc));
            return;
        }
        let ty_idx = ui.get_tunnel_edit_type_index();
        let is_domain = ty_idx == 2 || ty_idx == 3;
        let is_socks5 = ty_idx == 4;
        let use_plugin = ty_idx == 3 && ui.get_tunnel_edit_plugin_tls_term();
        let local_port = ui.get_tunnel_edit_local_port();
        let remote_port = ui.get_tunnel_edit_remote_port();

        if is_socks5 {
            if let Err(msg) = require_port_field(
                remote_port.as_str(),
                i18n::tunnel_remote_port_required(loc),
                i18n::tunnel_remote_port_invalid(loc),
            ) {
                toast_err(&ui, msg);
                return;
            }
            if ui.get_tunnel_edit_plugin_username().trim().is_empty() {
                toast_err(&ui, i18n::tunnel_plugin_username_required(loc));
                return;
            }
            if ui.get_tunnel_edit_plugin_password().trim().is_empty() {
                toast_err(&ui, i18n::tunnel_plugin_password_required(loc));
                return;
            }
        } else if is_domain {
            if ui.get_tunnel_edit_domains().trim().is_empty() {
                toast_err(&ui, i18n::tunnel_domain_required(loc));
                return;
            }
            if use_plugin {
                if ui.get_tunnel_edit_plugin_local_addr().trim().is_empty() {
                    toast_err(&ui, i18n::tunnel_plugin_addr_required(loc));
                    return;
                }
            } else if let Err(msg) = require_port_field(
                local_port.as_str(),
                i18n::tunnel_local_port_required(loc),
                i18n::tunnel_local_port_invalid(loc),
            ) {
                toast_err(&ui, msg);
                return;
            }
        } else {
            if let Err(msg) = require_port_field(
                local_port.as_str(),
                i18n::tunnel_local_port_required(loc),
                i18n::tunnel_local_port_invalid(loc),
            ) {
                toast_err(&ui, msg);
                return;
            }
            if let Err(msg) = require_port_field(
                remote_port.as_str(),
                i18n::tunnel_remote_port_required(loc),
                i18n::tunnel_remote_port_invalid(loc),
            ) {
                toast_err(&ui, msg);
                return;
            }
        }

        let row = collect_tunnel_form(&ui);
        let model = ui.get_tunnels();
        let previous: Vec<TunnelRow> = (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .collect();
        let mut rows = previous.clone();
        let edit_idx = ui.get_tunnel_editing_index();
        if edit_idx >= 0 {
            let i = edit_idx as usize;
            if rows
                .iter()
                .enumerate()
                .any(|(idx, p)| idx != i && p.name == row.name)
            {
                toast_err(&ui, i18n::tunnel_name_exists(loc));
                return;
            }
            if i < rows.len() {
                rows[i] = row;
            }
        } else {
            if rows.iter().any(|p| p.name == row.name) {
                toast_err(&ui, i18n::tunnel_name_exists(loc));
                return;
            }
            rows.push(row);
        }

        match persist_tunnels(&ui, &rows) {
            Ok(()) => {
                if !runtime::status().is_active() {
                    retain_remote_addrs(&mut rows, &previous);
                }
                ui.set_tunnels(slint::ModelRc::new(slint::VecModel::from(rows)));
                *remotes_gen_save.lock().unwrap_or_else(|e| e.into_inner()) = 0;
                ui.set_tunnel_editor_open(false);
                ui.set_tunnel_editing_index(-1);
                if !runtime::status().is_active() {
                    push_log(&ui, "INFO  tunnels saved");
                }
            }
            Err(e) => {
                toast_err(&ui, i18n::tunnel_persist_failed(loc, &e));
                push_log(&ui, &format!("ERROR failed to save tunnels: {e}"));
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_tunnel_edit(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let model = ui.get_tunnels();
        let Some(row) = model.row_data(index as usize) else {
            return;
        };
        ui.set_tunnel_editing_index(index);
        fill_tunnel_form(&ui, &row);
        ui.set_tunnel_editor_open(true);
    });

    let ui_weak = ui.as_weak();
    let remotes_gen_del = remotes_gen.clone();
    ui.on_tunnel_delete(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let loc = locale_of(&ui);
        let model = ui.get_tunnels();
        let previous: Vec<TunnelRow> = (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .collect();
        let mut rows: Vec<TunnelRow> = previous
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index as usize)
            .map(|(_, r)| r.clone())
            .collect();

        match persist_tunnels(&ui, &rows) {
            Ok(()) => {
                if !runtime::status().is_active() {
                    retain_remote_addrs(&mut rows, &previous);
                }
                ui.set_tunnels(slint::ModelRc::new(slint::VecModel::from(rows)));
                *remotes_gen_del.lock().unwrap_or_else(|e| e.into_inner()) = 0;
                if !runtime::status().is_active() {
                    push_log(&ui, "INFO  tunnel deleted");
                }
            }
            Err(e) => {
                toast_err(&ui, i18n::tunnel_persist_failed(loc, &e));
                push_log(&ui, &format!("ERROR failed to delete tunnel: {e}"));
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_tunnel_copy_remote(move |text| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let loc = locale_of(&ui);
        if copy_to_clipboard(text.as_str()) {
            toast_ok(&ui, i18n::tunnel_copied(loc));
        } else {
            toast_err(&ui, i18n::tunnel_copy_failed(loc));
        }
    });

    let ui_weak = ui.as_weak();
    let remotes_gen_cfg = remotes_gen.clone();
    ui.on_config_save(move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let loc = locale_of(&ui);
        if ui.get_server_addr().trim().is_empty() {
            toast_err(&ui, i18n::config_addr_required(loc));
            return;
        }
        if let Err(msg) = require_port_field(
            ui.get_server_port().as_str(),
            i18n::config_port_required(loc),
            i18n::config_port_invalid(loc),
        ) {
            toast_err(&ui, msg);
            return;
        }
        match persist_server_config(&ui) {
            Ok(()) => {
                if runtime::status().is_active() {
                    *remotes_gen_cfg.lock().unwrap_or_else(|e| e.into_inner()) = 0;
                } else {
                    toast_ok(&ui, i18n::config_saved(loc));
                    push_log(&ui, "INFO  config saved");
                }
            }
            Err(e) => {
                toast_err(&ui, i18n::config_persist_failed(loc, &e));
                push_log(&ui, &format!("ERROR failed to save config: {e}"));
            }
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_pick_file(move |target| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        pick_file::pick_into(&ui, target.as_str());
    });

    let ui_weak = ui.as_weak();
    ui.on_config_reset(move || {
        if let Some(ui) = ui_weak.upgrade() {
            let loc = locale_of(&ui);
            ui.set_server_addr("127.0.0.1".into());
            ui.set_server_port("9527".into());
            ui.set_token("".into());
            ui.set_user("".into());
            ui.set_protocol_index(0);
            ui.set_pool_count("1".into());
            ui.set_ws_path("".into());
            ui.set_tcp_mux(true);
            ui.set_tls_enable(true);
            ui.set_config_mux_keepalive("30".into());
            ui.set_config_heartbeat_interval("".into());
            ui.set_config_heartbeat_timeout("".into());
            ui.set_config_udp_packet_size("1500".into());
            ui.set_config_tls_server_name("".into());
            ui.set_config_tls_ca("".into());
            ui.set_config_tls_cert("".into());
            ui.set_config_tls_key("".into());
            ui.set_config_quic_keepalive("10".into());
            ui.set_config_quic_idle("30".into());
            ui.set_config_quic_streams("100000".into());
            ui.set_config_show_advanced(false);
            ui.set_config_file_path(
                config_bridge::path_display(&config_bridge::default_config_path()).into(),
            );
            toast_ok(&ui, i18n::config_reset(loc));
            push_log(&ui, "INFO  config reset to defaults");
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_logs_clear(move || {
        if let Some(ui) = ui_weak.upgrade() {
            {
                let mut store = log_store().lock().unwrap_or_else(|e| e.into_inner());
                store.clear();
                set_log_ui_cursor(store.cursor_after_full_sync());
            }
            ui.set_log_lines(slint::ModelRc::from(log_buffer::make_model()));
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_log_query_edited(move |q| {
        if let Some(ui) = ui_weak.upgrade() {
            if ui.get_page() != AppPage::Logger {
                return;
            }
            let store = log_store().lock().unwrap_or_else(|e| e.into_inner());
            ui.set_log_lines(slint::ModelRc::from(store.to_model(q.as_ref())));
            set_log_ui_cursor(store.cursor_after_full_sync());
            bump_log_scroll(&ui);
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_locale_changed(move |index| {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        ui.global::<Tr>().set_locale_index(index);
        if let Some(tray) = tray_weak.upgrade() {
            sync_tray_locale(&tray, index);
        }
        let label = if index == 0 { "zh-CN" } else { "en-US" };
        push_log(&ui, &format!("INFO  locale set to {label}"));
        persist_prefs(&ui);
    });

    let ui_weak = ui.as_weak();
    ui.on_theme_changed(move |index| {
        if let Some(ui) = ui_weak.upgrade() {
            apply_theme(&ui, index);
            let label = if index == 1 { "dark" } else { "light" };
            push_log(&ui, &format!("INFO  theme set to {label}"));
            persist_prefs(&ui);
        }
    });
}

fn log_store() -> &'static Mutex<LogStore> {
    static STORE: OnceLock<Mutex<LogStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(LogStore::default()))
}

fn log_ui_cursor() -> &'static Mutex<UiSyncCursor> {
    static CURSOR: OnceLock<Mutex<UiSyncCursor>> = OnceLock::new();
    CURSOR.get_or_init(|| Mutex::new(UiSyncCursor::default()))
}

fn reset_log_ui_cursor() {
    *log_ui_cursor().lock().unwrap_or_else(|e| e.into_inner()) = UiSyncCursor::default();
}

fn set_log_ui_cursor(cursor: UiSyncCursor) {
    *log_ui_cursor().lock().unwrap_or_else(|e| e.into_inner()) = cursor;
}

fn push_log(ui: &AppWindow, line: &str) {
    push_logs(ui, &[line.to_string()]);
}

fn push_logs(ui: &AppWindow, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    log_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_many_raw(lines);
    if ui.get_page() == AppPage::Logger {
        schedule_log_flush(ui);
    }
}

fn flush_logs_to_ui(ui: &AppWindow) {
    let query = ui.get_log_query();
    let store = log_store().lock().unwrap_or_else(|e| e.into_inner());
    let cursor = *log_ui_cursor().lock().unwrap_or_else(|e| e.into_inner());
    let model = ui.get_log_lines();
    if let Some(vm) = log_buffer::as_vec_model(&model) {
        if let Some(next) = store.append_to_model(vm, query.as_ref(), cursor) {
            set_log_ui_cursor(next);
            bump_log_scroll(ui);
            return;
        }
    }
    ui.set_log_lines(slint::ModelRc::from(store.to_model(query.as_ref())));
    set_log_ui_cursor(store.cursor_after_full_sync());
    bump_log_scroll(ui);
}

fn schedule_log_flush(ui: &AppWindow) {
    static PENDING: AtomicBool = AtomicBool::new(false);
    if PENDING.swap(true, Ordering::Relaxed) {
        return;
    }
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        PENDING.store(false, Ordering::Relaxed);
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if ui.get_page() == AppPage::Logger {
            flush_logs_to_ui(&ui);
        }
    });
}

fn bump_log_scroll(ui: &AppWindow) {
    if !ui.get_log_auto_scroll() {
        return;
    }
    if ui.get_page() != AppPage::Logger {
        return;
    }

    static PENDING: AtomicBool = AtomicBool::new(false);
    if PENDING.swap(true, Ordering::Relaxed) {
        return;
    }

    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(16), move || {
        PENDING.store(false, Ordering::Relaxed);
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        if ui.get_log_auto_scroll() && ui.get_page() == AppPage::Logger {
            ui.set_log_scroll_gen(ui.get_log_scroll_gen().wrapping_add(1));
        }
    });
}
