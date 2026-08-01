use crate::types::Connection;
use anyhow::{anyhow, Context, Result};
use rusb::{Direction, TransferType, UsbContext};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

fn parse_hex_u16(label: &str, value: &str) -> Result<u16> {
    let clean = value.trim().trim_start_matches("0x");
    u16::from_str_radix(clean, 16).with_context(|| format!("{label}='{value}' no es un hex válido"))
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

    let handle = context
        .open_device_with_vid_pid(vendor_id, product_id)
        .ok_or_else(|| {
            // Diagnóstico: ¿libusb puede ver el dispositivo pero no abrirlo?
            let mut visible = false;
            if let Ok(devices) = context.devices() {
                for dev in devices.iter() {
                    if let Ok(desc) = dev.device_descriptor() {
                        if desc.vendor_id() == vendor_id && desc.product_id() == product_id {
                            visible = true;
                            break;
                        }
                    }
                }
            }
            if visible {
                anyhow!(
                    "USB {:04x}:{:04x} detectada pero no accesible. Instalá el driver WinUSB con Zadig (zadig.akeo.ie), seleccioná la impresora y hacé clic en 'Replace Driver'. Luego desconectá y reconectá la impresora.",
                    vendor_id,
                    product_id
                )
            } else {
                anyhow!(
                    "no se encontró impresora USB con vendor={:04x} product={:04x}",
                    vendor_id,
                    product_id
                )
            }
        })?;

    // Auto-detach del kernel driver en todas las plataformas (Linux, macOS, Windows)
    // libusb 1.0.24+ soporta esto en Windows via WinUSB.
    let _ = handle.set_auto_detach_kernel_driver(true);

    // En macOS, el kernel driver (IOKit) a veces no se suelta con auto-detach.
    // Intentar detach explícito en todas las interfaces disponibles.
    let device = handle.device();
    let device_desc = device
        .device_descriptor()
        .context("no se pudo leer el descriptor del dispositivo USB")?;

    // Detach explícito del kernel driver para macOS
    for cfg_idx in 0..device_desc.num_configurations() {
        if let Ok(cfg) = device.config_descriptor(cfg_idx) {
            for iface in cfg.interfaces() {
                for desc in iface.descriptors() {
                    let _ = handle.detach_kernel_driver(desc.interface_number());
                }
            }
        }
    }

    // Probar todas las configuraciones disponibles — no solo la activa.
    // Algunos dispositivos compuestos requieren cambiar de configuración.
    let num_configs = device_desc.num_configurations();
    log::debug!(
        "USB vid={:04x} pid={:04x}: {} configuración(es) disponible(s)",
        vendor_id,
        product_id,
        num_configs
    );

    let mut found: Option<(u8, u8, TransferType, u8)> = None;

    for cfg_idx in 0..num_configs {
        // Intentar leer descriptor de esta configuración
        let cfg_desc = match device.config_descriptor(cfg_idx) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Si esta no es la activa, intentar setearla
        let active = device.active_config_descriptor().ok();
        let needs_switch = active
            .as_ref()
            .is_none_or(|a| a.number() != cfg_desc.number());

        if needs_switch {
            log::debug!(
                "USB: cambiando a configuración {} (bNumInterfaces={})",
                cfg_desc.number(),
                cfg_desc.num_interfaces()
            );
            // Desclaim cualquier interfaz previa primero
            let _ = handle.set_active_configuration(cfg_desc.number());
        }

        // Escanear interfaces, alternate settings y endpoints en esta configuración
        for iface in cfg_desc.interfaces() {
            for desc in iface.descriptors() {
                for ep in desc.endpoint_descriptors() {
                    if ep.direction() != Direction::Out {
                        continue;
                    }
                    match ep.transfer_type() {
                        TransferType::Bulk => {
                            found = Some((
                                desc.interface_number(),
                                ep.address(),
                                TransferType::Bulk,
                                cfg_desc.number(),
                            ));
                            break;
                        }
                        TransferType::Interrupt if found.is_none() => {
                            found = Some((
                                desc.interface_number(),
                                ep.address(),
                                TransferType::Interrupt,
                                cfg_desc.number(),
                            ));
                        }
                        _ => {}
                    }
                }
                if matches!(found.as_ref(), Some((_, _, TransferType::Bulk, _))) {
                    break;
                }
            }
            if matches!(found.as_ref(), Some((_, _, TransferType::Bulk, _))) {
                break;
            }
        }
        if matches!(found.as_ref(), Some((_, _, TransferType::Bulk, _))) {
            break;
        }
    }

    let (iface, endpoint, transfer_type, _cfg_num) =
        found.ok_or_else(|| anyhow!("la impresora no expone endpoint bulk ni interrupt OUT"))?;

    // set_auto_detach_kernel_driver(true) ya se llamó arriba — cubre todas las plataformas.
    // El claim_interface automáticamente hará detach si es necesario.

    handle
        .claim_interface(iface)
        .with_context(|| format!("no se pudo reclamar la interfaz USB {iface}"))?;

    // Pequeña pausa para que el dispositivo se estabilice después del detach
    std::thread::sleep(std::time::Duration::from_millis(200));

    let written = match transfer_type {
        TransferType::Bulk => {
            let result = handle.write_bulk(endpoint, bytes, TIMEOUT);
            // Si falla, un solo retry después de 500ms
            if result.is_err() {
                std::thread::sleep(std::time::Duration::from_millis(500));
                handle
                    .write_bulk(endpoint, bytes, TIMEOUT)
                    .context("error escribiendo bulk al endpoint USB")?
            } else {
                result.context("error escribiendo bulk al endpoint USB")?
            }
        }
        TransferType::Interrupt => handle
            .write_interrupt(endpoint, bytes, TIMEOUT)
            .context("error escribiendo interrupt al endpoint USB")?,
        _ => unreachable!(),
    };

    let _ = handle.release_interface(iface);

    log::debug!(
        "USB OK (vid={:04x} pid={:04x}, {} bytes, {:?})",
        vendor_id,
        product_id,
        written,
        transfer_type
    );

    if written < bytes.len() {
        return Err(anyhow!(
            "escritura USB parcial: {written} de {} bytes",
            bytes.len()
        ));
    }
    Ok(())
}
