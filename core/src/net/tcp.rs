use socket2::{SockRef, TcpKeepalive};
use std::time::Duration;
use tokio::net::TcpStream;

pub const DEFAULT_TCP_KEEPALIVE_IDLE_SECS: u64 = 30;

pub const DEFAULT_TCP_KEEPALIVE_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpKeepaliveConfig {
    pub idle_secs: u64,
    pub interval_secs: u64,
}

impl Default for TcpKeepaliveConfig {
    fn default() -> Self {
        Self {
            idle_secs: DEFAULT_TCP_KEEPALIVE_IDLE_SECS,
            interval_secs: DEFAULT_TCP_KEEPALIVE_INTERVAL_SECS,
        }
    }
}

impl TcpKeepaliveConfig {
    pub fn new(idle_secs: u64, interval_secs: u64) -> Self {
        Self {
            idle_secs,
            interval_secs: if idle_secs > 0 && interval_secs == 0 {
                DEFAULT_TCP_KEEPALIVE_INTERVAL_SECS
            } else {
                interval_secs
            },
        }
    }

    pub fn enabled(self) -> bool {
        self.idle_secs > 0
    }
}

#[inline]
pub fn tune_tcp_stream(stream: &TcpStream, keepalive: TcpKeepaliveConfig) {
    enable_nodelay(stream);
    enable_keepalive(stream, keepalive);
}

#[inline]
pub fn enable_nodelay(stream: &TcpStream) {
    if let Err(e) = stream.set_nodelay(true) {
        tracing::trace!(error = %e, "tcp set_nodelay failed");
    }
}

#[inline]
pub fn enable_keepalive(stream: &TcpStream, config: TcpKeepaliveConfig) {
    if !config.enabled() {
        return;
    }
    let sock = SockRef::from(stream);
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(config.idle_secs))
        .with_interval(Duration::from_secs(config.interval_secs.max(1)));
    if let Err(e) = sock.set_tcp_keepalive(&keepalive) {
        tracing::trace!(error = %e, "tcp set_keepalive failed");
    }
}
