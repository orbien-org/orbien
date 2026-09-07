use super::{ConnectionInfo, Plugin, PluginContext};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use httparse::Status;
use orbien_core::config::PluginConfig;
use orbien_core::tls::load_or_generate_https_server_config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

pub struct TlsTermPlugin {
    local_addr: String,
    host_header_rewrite: String,
    acceptor: TlsAcceptor,
}

impl TlsTermPlugin {
    pub fn new(ctx: PluginContext, cfg: &PluginConfig) -> Result<Self> {
        let local_addr = cfg.service.trim().to_string();
        if local_addr.is_empty() {
            bail!("tls-term requires plugin.service (e.g. \"127.0.0.1:80\")");
        }

        let cn = if ctx.cert_common_name.is_empty() {
            "localhost".to_string()
        } else {
            ctx.cert_common_name.clone()
        };
        let tls_cfg = load_or_generate_https_server_config(&cfg.cert_file, &cfg.key_file, &cn)?;
        let acceptor = TlsAcceptor::from(tls_cfg);

        tracing::info!(
            tunnel = %ctx.name,
            %local_addr,
            rewrite = %cfg.host_header_rewrite,
            "plugin tls-term ready (TLS terminates on agent)"
        );

        Ok(Self {
            local_addr,
            host_header_rewrite: cfg.host_header_rewrite.clone(),
            acceptor,
        })
    }
}

#[async_trait]
impl Plugin for TlsTermPlugin {
    fn name(&self) -> &str {
        "tls-term"
    }

    async fn handle(&self, conn: ConnectionInfo) -> Result<()> {
        let tls = self
            .acceptor
            .accept(conn.stream)
            .await
            .map_err(|e| anyhow!("tls-term TLS accept failed: {e}"))?;

        let mut local = TcpStream::connect(&self.local_addr)
            .await
            .map_err(|e| anyhow!("tls-term dial {}: {e}", self.local_addr))?;
        orbien_core::net::enable_nodelay(&local);

        let (mut tls_r, mut tls_w) = tokio::io::split(tls);
        let (mut headers, body_prefix) = read_http_request_head(&mut tls_r).await?;
        apply_host_rewrite(&mut headers, &self.host_header_rewrite)?;
        orbien_core::net::apply_x_forwarded_for(&mut headers, &conn.src_addr, "https")?;

        local.write_all(&headers).await?;
        if !body_prefix.is_empty() {
            local.write_all(&body_prefix).await?;
        }

        tracing::debug!(
            local = %self.local_addr,
            src = %format!("{}:{}", conn.src_addr, conn.src_port),
            "tls-term joining decrypted <-> local HTTP"
        );

        let (mut local_r, mut local_w) = tokio::io::split(local);
        let _ = tokio::try_join!(
            async move { tokio::io::copy(&mut local_r, &mut tls_w).await },
            async move { tokio::io::copy(&mut tls_r, &mut local_w).await },
        );
        Ok(())
    }
}

async fn read_http_request_head<R: AsyncReadExt + Unpin>(
    stream: &mut R,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            bail!("client closed before http headers completed");
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 64 * 1024 {
            bail!("http headers too large");
        }
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf)? {
            Status::Complete(header_len) => {
                if header_len > buf.len() {
                    bail!("httparse header length exceeds buffer");
                }
                let body_prefix = buf.split_off(header_len);
                return Ok((buf, body_prefix));
            }
            Status::Partial => continue,
        }
    }
}

fn header_block_end(lines: &[String]) -> usize {
    lines
        .iter()
        .position(|l| l == "\r\n" || l == "\n")
        .unwrap_or(lines.len())
}

fn apply_host_rewrite(buf: &mut Vec<u8>, host_rewrite: &str) -> Result<()> {
    if host_rewrite.is_empty() {
        return Ok(());
    }

    let text = String::from_utf8_lossy(buf);
    let mut lines: Vec<String> = text.split_inclusive('\n').map(|s| s.to_string()).collect();
    if lines.is_empty() {
        bail!("empty http request");
    }

    let ending = if lines.iter().any(|l| l.ends_with("\r\n")) {
        "\r\n"
    } else {
        "\n"
    };

    let blank_idx = header_block_end(&lines);
    let mut replaced = false;
    for line in lines.iter_mut().take(blank_idx).skip(1) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.len() >= 5 && trimmed.as_bytes()[..5].eq_ignore_ascii_case(b"host:") {
            *line = format!("Host: {host_rewrite}{ending}");
            replaced = true;
            break;
        }
    }
    if !replaced {
        lines.insert(blank_idx, format!("Host: {host_rewrite}{ending}"));
    }

    *buf = lines.join("").into_bytes();
    Ok(())
}
