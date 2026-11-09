use super::stream::{boxed_stream, DynStream};
use anyhow::{bail, Context, Result};
use rcgen::{CertificateParams, KeyPair, SanType};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub const ALPN_ORBIEN: &[u8] = b"orbien";
pub const TLS_HANDSHAKE_TYPE: u8 = 0x16;

pub struct GeneratedCert {
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivatePkcs8KeyDer<'static>,
}

pub fn generate_self_signed_cert(common_name: &str) -> Result<GeneratedCert> {
    let mut params = CertificateParams::new(vec![common_name.to_string()])?;
    params
        .subject_alt_names
        .push(SanType::DnsName(common_name.try_into()?));
    params
        .subject_alt_names
        .push(SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::new(127, 0, 0, 1),
        )));

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(key_pair.serialize_der());

    Ok(GeneratedCert {
        certs: vec![cert_der],
        key: key_der,
    })
}

#[derive(Debug)]
pub struct SkipServerVerification;

impl SkipServerVerification {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn install_ring_provider() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Ok(())
}

pub fn load_pem_cert_key(
    cert_file: &str,
    key_file: &str,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let mut cert_reader = BufReader::new(
        File::open(Path::new(cert_file)).with_context(|| format!("open certFile {cert_file}"))?,
    );
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("parse certificate PEM")?
        .into_iter()
        .map(CertificateDer::from)
        .collect();
    if certs.is_empty() {
        bail!("no certificates in {cert_file}");
    }

    let mut key_reader = BufReader::new(
        File::open(Path::new(key_file)).with_context(|| format!("open keyFile {key_file}"))?,
    );
    let key = rustls_pemfile::private_key(&mut key_reader)
        .context("parse private key PEM")?
        .ok_or_else(|| anyhow::anyhow!("no private key in {key_file}"))?;

    Ok((certs, key))
}

fn load_ca_roots(ca_path: &str) -> Result<RootCertStore> {
    let mut reader = BufReader::new(
        File::open(Path::new(ca_path)).with_context(|| format!("open trustedCaFile {ca_path}"))?,
    );
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .context("parse CA PEM")?
        .into_iter()
        .map(CertificateDer::from)
        .collect();
    if certs.is_empty() {
        bail!("no CA certificates in {ca_path}");
    }
    let mut roots = RootCertStore::empty();
    for c in certs {
        roots
            .add(c)
            .map_err(|e| anyhow::anyhow!("add CA cert: {e}"))?;
    }
    Ok(roots)
}

pub fn load_server_cert_key(cert_file: &str, key_file: &str) -> Result<Arc<ServerConfig>> {
    install_ring_provider()?;
    let cert = cert_file.trim();
    let key = key_file.trim();
    if cert.is_empty() || key.is_empty() {
        bail!("certFile and keyFile are required");
    }
    let (certs, key) = load_pem_cert_key(cert, key)?;
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("build rustls ServerConfig")?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

pub fn new_server_tls_config(
    cert_file: &str,
    key_file: &str,
    ca_path: &str,
) -> Result<Arc<ServerConfig>> {
    install_ring_provider()?;

    let (certs, key) = if cert_file.trim().is_empty() || key_file.trim().is_empty() {
        tracing::info!(
            "transport.tls: no certFile/keyFile — generating ephemeral self-signed cert"
        );
        let gen = generate_self_signed_cert("orbien-server")?;
        (gen.certs, PrivateKeyDer::Pkcs8(gen.key))
    } else {
        load_pem_cert_key(cert_file, key_file)?
    };

    let builder = if ca_path.trim().is_empty() {
        ServerConfig::builder().with_no_client_auth()
    } else {
        let roots = load_ca_roots(ca_path)?;
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .context("build client cert verifier")?;
        ServerConfig::builder().with_client_cert_verifier(verifier)
    };

    let cfg = builder
        .with_single_cert(certs, key)
        .context("build rustls ServerConfig")?;

    Ok(Arc::new(cfg))
}

pub fn new_client_tls_config(
    cert_file: &str,
    key_file: &str,
    ca_path: &str,
) -> Result<Arc<ClientConfig>> {
    install_ring_provider()?;

    let builder = if ca_path.trim().is_empty() {
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(SkipServerVerification::new())
    } else {
        let roots = load_ca_roots(ca_path)?;
        let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
            .build()
            .context("build server cert verifier")?;
        ClientConfig::builder().with_webpki_verifier(verifier)
    };

    let cfg = if !cert_file.trim().is_empty() && !key_file.trim().is_empty() {
        let (certs, key) = load_pem_cert_key(cert_file, key_file)?;
        builder
            .with_client_auth_cert(certs, key)
            .context("load client certificate")?
    } else {
        builder.with_no_client_auth()
    };

    Ok(Arc::new(cfg))
}

pub fn server_crypto_from_tls_files(
    cert_file: &str,
    key_file: &str,
    ca_path: &str,
) -> Result<quinn::crypto::rustls::QuicServerConfig> {
    let mut cfg = (*new_server_tls_config(cert_file, key_file, ca_path)?).clone();
    cfg.alpn_protocols = vec![ALPN_ORBIEN.to_vec()];

    quinn::crypto::rustls::QuicServerConfig::try_from(cfg)
        .map_err(|e| anyhow::anyhow!("QuicServerConfig: {e}"))
}

pub fn client_crypto_from_tls_files(
    cert_file: &str,
    key_file: &str,
    ca_path: &str,
) -> Result<quinn::crypto::rustls::QuicClientConfig> {
    let mut cfg = (*new_client_tls_config(cert_file, key_file, ca_path)?).clone();
    cfg.alpn_protocols = vec![ALPN_ORBIEN.to_vec()];
    quinn::crypto::rustls::QuicClientConfig::try_from(cfg)
        .map_err(|e| anyhow::anyhow!("QuicClientConfig: {e}"))
}

pub fn server_crypto(
    certs: Vec<CertificateDer<'static>>,
    key: PrivatePkcs8KeyDer<'static>,
) -> Result<quinn::crypto::rustls::QuicServerConfig> {
    install_ring_provider()?;
    let mut cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key.into())
        .context("build rustls ServerConfig")?;
    cfg.alpn_protocols = vec![ALPN_ORBIEN.to_vec()];
    quinn::crypto::rustls::QuicServerConfig::try_from(cfg)
        .map_err(|e| anyhow::anyhow!("QuicServerConfig: {e}"))
}

pub fn client_crypto_insecure() -> Result<quinn::crypto::rustls::QuicClientConfig> {
    client_crypto_from_tls_files("", "", "")
}

pub async fn client_enable_tls(
    stream: DynStream,
    tls_cfg: Arc<ClientConfig>,
    server_name: &str,
) -> Result<DynStream> {
    let name = ServerName::try_from(server_name.to_owned())
        .map_err(|e| anyhow::anyhow!("invalid tls serverName {server_name}: {e}"))?;
    let connector = TlsConnector::from(tls_cfg);
    let tls = connector
        .connect(name, stream)
        .await
        .context("client TLS handshake")?;
    Ok(boxed_stream(tls))
}

pub async fn check_and_enable_tls(
    mut stream: DynStream,
    tls_cfg: Arc<ServerConfig>,
    force: bool,
) -> Result<DynStream> {
    let mut first = [0u8; 1];

    stream
        .read_exact(&mut first)
        .await
        .context("peek TLS first byte")?;

    match first[0] {
        TLS_HANDSHAKE_TYPE => {
            let stream = PrefixedByteStream {
                prefix: Some(first[0]),
                inner: stream,
            };
            let acceptor = TlsAcceptor::from(tls_cfg);
            let tls = acceptor
                .accept(boxed_stream(stream))
                .await
                .context("server TLS handshake")?;
            Ok(boxed_stream(tls))
        }
        _ if force => {
            bail!(
                "transport.tls.force=true but first byte is 0x{:02x} (expected TLS handshake 0x16)",
                first[0]
            );
        }
        _ => Ok(boxed_stream(PrefixedByteStream {
            prefix: Some(first[0]),
            inner: stream,
        })),
    }
}

struct PrefixedByteStream {
    prefix: Option<u8>,
    inner: DynStream,
}

impl AsyncRead for PrefixedByteStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Some(b) = self.prefix.take() {
            if buf.remaining() > 0 {
                buf.put_slice(&[b]);
                return std::task::Poll::Ready(Ok(()));
            }
            self.prefix = Some(b);
        }
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedByteStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
