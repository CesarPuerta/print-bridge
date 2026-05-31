use crate::types::{Connection, ConnectionType};
use anyhow::{anyhow, Result};

pub mod tcp;
pub mod usb;
pub mod serial;

/// Despacha los bytes ESC/POS al adapter correspondiente.
pub fn send_bytes(connection: &Connection, bytes: &[u8]) -> Result<()> {
    match connection.conn_type {
        ConnectionType::Network => tcp::send(connection, bytes),
        ConnectionType::Usb => usb::send(connection, bytes),
        ConnectionType::Serial => serial::send(connection, bytes),
        ConnectionType::Bluetooth => Err(anyhow!(
            "Bluetooth aún no está soportado en esta versión del Print Bridge"
        )),
    }
}
