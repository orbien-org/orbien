mod kcp;
mod quic;
mod stream;
mod tls;
mod websocket;
mod yamux_mux;

pub use kcp::{accept_kcp, bind_kcp_listener, default_kcp_config, dial_kcp};
pub use quic::{build_client_endpoint, build_server_endpoint, quic_bi, QuicBiStream, QuicSession};
pub use stream::{boxed_stream, AsyncStream, DynStream};
pub use tls::{
    check_and_enable_tls, client_crypto_from_tls_files, client_crypto_insecure, client_enable_tls,
    generate_self_signed_cert, install_ring_provider, load_pem_cert_key, new_client_tls_config,
    new_server_tls_config, server_crypto, server_crypto_from_tls_files, ALPN_ORBIEN,
};
pub use websocket::{
    accept_websocket, dial_websocket, is_websocket_http_request, normalize_ws_path, WsByteStream,
    ORBIEN_WEBSOCKET_PATH,
};
pub use yamux_mux::{serve_yamux_session, YamuxClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Quic,
    Websocket,
    Kcp,
}

impl Protocol {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("tcp") {
            Some(Self::Tcp)
        } else if s.eq_ignore_ascii_case("quic") {
            Some(Self::Quic)
        } else if s.eq_ignore_ascii_case("websocket") || s.eq_ignore_ascii_case("ws") {
            Some(Self::Websocket)
        } else if s.eq_ignore_ascii_case("kcp") {
            Some(Self::Kcp)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Quic => "quic",
            Self::Websocket => "websocket",
            Self::Kcp => "kcp",
        }
    }

    pub fn supports_yamux(self) -> bool {
        matches!(self, Self::Tcp | Self::Websocket | Self::Kcp)
    }
}
