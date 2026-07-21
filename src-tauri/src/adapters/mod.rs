use crate::types::{Connection, ConnectionType};
use anyhow::{anyhow, Result};

pub mod serial;
pub mod tcp;
#[cfg(not(windows))]
pub mod usb;
#[cfg(windows)]
pub mod win_usb;

/// Despacha los bytes ESC/POS al adapter correspondiente.
pub fn send_bytes(connection: &Connection, bytes: &[u8]) -> Result<()> {
    match connection.conn_type {
        ConnectionType::Network => tcp::send(connection, bytes),
        ConnectionType::Usb => {
            #[cfg(windows)]
            {
                win_usb::send(connection, bytes)
            }
            #[cfg(not(windows))]
            {
                usb::send(connection, bytes)
            }
        }
        ConnectionType::Serial => serial::send(connection, bytes),
        ConnectionType::Bluetooth => Err(anyhow!(
            "Bluetooth aún no está soportado en esta versión del Print Bridge"
        )),
    }
}
