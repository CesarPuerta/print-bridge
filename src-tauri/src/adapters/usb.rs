use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use rusb::{Direction, TransferType, UsbContext};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

fn parse_hex_u16(label: &str, value: &str) -> Result<u16> {
    let clean = value.trim().trim_start_matches("0x");
    u16::from_str_radix(clean, 16)
        .with_context(|| format!("{label}='{value}' no es un hex válido"))
}

pub fn send(conn: &Connection, bytes: &[u8]) -> Result<()> {
    let vendor_id = parse_hex_u16(
        "vendorId",
        conn.vendor_id
            .as_deref()
            .ok_or_else(|| anyhow!("connection.vendorId requerido para USB"))?,
    )?;
    let product_id = parse_hex_u16(
        "productId",
        conn.product_id
            .as_deref()
            .ok_or_else(|| anyhow!("connection.productId requerido para USB"))?,
    )?;

    let context = rusb::Context::new().context("no se pudo inicializar libusb")?;

    let mut handle = context
        .open_device_with_vid_pid(vendor_id, product_id)
        .ok_or_else(|| {
            anyhow!(
                "no se encontró impresora USB con vendor={:04x} product={:04x}",
                vendor_id,
                product_id
            )
        })?;

    let device = handle.device();
    let config = device
        .active_config_descriptor()
        .context("no se pudo leer la configuración activa")?;

    // Buscar primer endpoint OUT bulk.
    let mut found: Option<(u8, u8)> = None;
    for interface in config.interfaces() {
        for desc in interface.descriptors() {
            for ep in desc.endpoint_descriptors() {
                if ep.direction() == Direction::Out && ep.transfer_type() == TransferType::Bulk {
                    found = Some((desc.interface_number(), ep.address()));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (iface, endpoint) = found.ok_or_else(|| anyhow!("la impresora no expone endpoint bulk OUT"))?;

    // En Linux/macOS hay que detach del kernel driver si lo tiene.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if handle.kernel_driver_active(iface).unwrap_or(false) {
            let _ = handle.detach_kernel_driver(iface);
        }
    }

    handle
        .claim_interface(iface)
        .with_context(|| format!("no se pudo reclamar la interfaz USB {iface}"))?;

    let written = handle
        .write_bulk(endpoint, bytes, TIMEOUT)
        .context("error escribiendo bulk al endpoint USB")?;

    let _ = handle.release_interface(iface);

    log::info!(
        "USB → vid={:04x} pid={:04x} ep={:#x} ({} bytes)",
        vendor_id,
        product_id,
        endpoint,
        written
    );

    if written < bytes.len() {
        return Err(anyhow!("escritura USB parcial: {written} de {} bytes", bytes.len()));
    }
    Ok(())
}
