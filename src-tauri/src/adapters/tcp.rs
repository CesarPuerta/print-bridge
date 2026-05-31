use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(8);

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

    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .with_context(|| format!("no se pudo conectar a {addr}"))?;
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .context("set_write_timeout falló")?;

    stream.write_all(bytes).context("error escribiendo a la impresora TCP")?;
    stream.flush().ok();

    log::info!("TCP → {} ({} bytes)", addr, bytes.len());
    Ok(())
}
