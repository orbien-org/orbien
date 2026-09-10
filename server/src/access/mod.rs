use orbien_core::tls::PrefixedStream;
use std::net::SocketAddr;
use tokio::net::TcpStream;

pub struct IngressConn {
    pub stream: PrefixedStream<TcpStream>,
    pub peer: SocketAddr,
    pub source: SocketAddr,
    pub local: Option<SocketAddr>,
}

pub fn prepare_ingress(stream: TcpStream, peer: SocketAddr) -> IngressConn {
    let local = stream.local_addr().ok();
    IngressConn {
        stream: PrefixedStream::new(Vec::new(), stream),
        peer,
        source: peer,
        local,
    }
}
