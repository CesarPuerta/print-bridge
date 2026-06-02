use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Bloquea path traversal y rutas no-dispositivo. Una impresora serial legítima
/// siempre está en /dev/tty*, /dev/cu.* (Unix) o COM* (Windows).
fn validate_serial_path(path: &str) -> Result<()> {
    if Path::new(path).components().any(|c| c.as_os_str() == "..") {
        return Err(anyhow!("path serial inválido (path traversal)"));
    }
    #[cfg(unix)]
    {
        if !path.starts_with("/dev/tty") && !path.starts_with("/dev/cu.") {
            return Err(anyhow!(
                "path serial inválido: {path} (solo /dev/tty* o /dev/cu.*)"
            ));
        }
    }
    #[cfg(windows)]
    {
        let up = path.to_uppercase();
        if !up.starts_with("COM") && !up.starts_with("\\\\.\\COM") {
            return Err(anyhow!("path serial inválido: {path} (solo COM*)"));
        }
    }
    Ok(())
}

pub fn send(conn: &Connection, bytes: &[u8]) -> Result<()> {
    let path = conn
        .path
        .as_deref()
        .ok_or_else(|| anyhow!("connection.path requerido para Serial"))?;
    validate_serial_path(path)?;
    let baud = conn.baud_rate.unwrap_or(9600);

    let mut port = serialport::new(path, baud)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .timeout(TIMEOUT)
        .open()
        .with_context(|| format!("no se pudo abrir puerto serial {path} @ {baud}"))?;

    port.write_all(bytes).context("error escribiendo al puerto serial")?;
    port.flush().ok();

    log::debug!("Serial OK ({} bytes)", bytes.len());
    Ok(())
}
