use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::io::Write;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

pub fn send(conn: &Connection, bytes: &[u8]) -> Result<()> {
    let path = conn
        .path
        .as_deref()
        .ok_or_else(|| anyhow!("connection.path requerido para Serial"))?;
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

    log::info!("Serial → {} @ {} ({} bytes)", path, baud, bytes.len());
    Ok(())
}
