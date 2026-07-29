/// Escaneo de impresoras USB conectadas al equipo.
/// Usa libusb (rusb) para enumerar dispositivos con clase de impresión
/// o VIDs conocidos de fabricantes POS.
use rusb::{Direction, UsbContext};
use serde::Serialize;

/// Información mínima de una impresora USB detectada.
#[derive(Debug, Clone, Serialize)]
pub struct UsbDeviceInfo {
    #[serde(rename = "vendorId")]
    pub vendor_id: String,
    #[serde(rename = "productId")]
    pub product_id: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    #[serde(rename = "serialNumber")]
    pub serial_number: Option<String>,
}

/// VIDs de fabricantes conocidos de impresoras térmicas POS.
const POS_VENDOR_IDS: &[u16] = &[
    0x04B8, // Seiko Epson
    0x0519, // Star Micronics
    0x1504, // BIXOLON
    0x248A, // Genérica china (POS-80, etc.)
    0x1FC9, // Genérica china
    0x0FE6, // Genérica china
    0x0416, // Winbond (algunas POS)
    0x6868, // Genérica china
    0x2D90, // Genérica
];

/// Clase USB 0x07 = Printer
const USB_CLASS_PRINTER: u8 = 0x07;

/// Escanea todos los dispositivos USB y devuelve los que parecen impresoras.
pub fn scan() -> Vec<UsbDeviceInfo> {
    let ctx = match rusb::Context::new() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("no se pudo inicializar libusb para escaneo: {e}");
            return Vec::new();
        }
    };

    let devices = match ctx.devices() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("no se pudieron enumerar dispositivos USB: {e}");
            return Vec::new();
        }
    };

    let mut found: Vec<UsbDeviceInfo> = Vec::new();

    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        let vid = desc.vendor_id();
        let pid = desc.product_id();

        // Aceptar cualquier dispositivo con endpoint OUT que pueda recibir datos.
        // Muchas impresoras POS chinas usan chips USB-serial que no reportan
        // clase de impresora (0x07). Ser permisivos evita falsos negativos.

        // Verificar que tenga al menos una interfaz con endpoint OUT.
        if !has_out_endpoint(&device) {
            continue;
        }

        // Leer strings del descriptor (mejor esfuerzo).
        let handle = match device.open() {
            Ok(h) => h,
            Err(_) => continue,
        };

        let languages = handle.read_languages(TIMEOUT).unwrap_or_default();
        let lang = languages.first().copied();

        let manufacturer = lang
            .and_then(|l| handle.read_manufacturer_string(l, &desc, TIMEOUT).ok())
            .filter(|s| !s.is_empty());

        let product = lang
            .and_then(|l| handle.read_product_string(l, &desc, TIMEOUT).ok())
            .filter(|s| !s.is_empty());

        let serial_number = lang
            .and_then(|l| handle.read_serial_number_string(l, &desc, TIMEOUT).ok())
            .filter(|s| !s.is_empty());

        // Evitar duplicados (mismo VID+PID).
        let vid_hex = format!("{:04X}", vid);
        let pid_hex = format!("{:04X}", pid);
        if found
            .iter()
            .any(|d| d.vendor_id == vid_hex && d.product_id == pid_hex)
        {
            continue;
        }

        found.push(UsbDeviceInfo {
            vendor_id: vid_hex,
            product_id: pid_hex,
            manufacturer,
            product,
            serial_number,
        });
    }

    // Ordenar: impresoras conocidas (POS) primero, luego alfabéticamente
    found.sort_by(|a, b| {
        let a_known = u16::from_str_radix(&a.vendor_id, 16)
            .map(|v| POS_VENDOR_IDS.contains(&v))
            .unwrap_or(false);
        let b_known = u16::from_str_radix(&b.vendor_id, 16)
            .map(|v| POS_VENDOR_IDS.contains(&v))
            .unwrap_or(false);
        b_known
            .cmp(&a_known)
            .then_with(|| a.product.cmp(&b.product))
    });

    found
}

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Verifica si el dispositivo tiene al menos una interfaz con endpoint OUT
/// (bulk o interrupt). Busca en todas las configuraciones y alternate settings.
fn has_out_endpoint(device: &rusb::Device<rusb::Context>) -> bool {
    let desc = match device.device_descriptor() {
        Ok(d) => d,
        Err(_) => return false,
    };

    for cfg_idx in 0..desc.num_configurations() {
        let cfg = match device.config_descriptor(cfg_idx) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for iface in cfg.interfaces() {
            for iface_desc in iface.descriptors() {
                for ep in iface_desc.endpoint_descriptors() {
                    if ep.direction() == Direction::Out {
                        return true;
                    }
                }
            }
        }
    }
    false
}
