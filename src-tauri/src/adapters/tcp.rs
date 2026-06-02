use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(8);

/// Bloquea IPs sensibles que podrían habilitar SSRF a la infraestructura
/// del sistema (link-local, multicast, metadata cloud, broadcast).
fn is_forbidden_addr(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            v4.is_link_local()                       // 169.254.0.0/16 (incl. metadata AWS/Azure/GCP)
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            v6.is_multicast()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

pub fn send(conn: &Connection, bytes: &[u8]) -> Result<()> {
    let host = conn
        .host
        .as_deref()
        .ok_or_else(|| anyhow!("connection.host requerido para tipo network"))?;
    let port = conn.port.unwrap_or(9100);

    let addr: SocketAddr = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("DNS lookup falló para {host}:{port}"))?
        .next()
        .ok_or_else(|| anyhow!("no se resolvió {host}:{port}"))?;

    if is_forbidden_addr(&addr.ip()) {
        return Err(anyhow!(
            "host {host} resuelve a una dirección no permitida ({})",
            addr.ip()
        ));
    }

    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .with_context(|| format!("no se pudo conectar a {addr}"))?;
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .context("set_write_timeout falló")?;

    stream.write_all(bytes).context("error escribiendo a la impresora TCP")?;
    stream.flush().ok();

    log::debug!("TCP OK ({} bytes)", bytes.len());
    Ok(())
}
