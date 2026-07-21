#[allow(unused_imports)]
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Config persistida en ~/.cegel/bridge.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub port: u16,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub paired_business_id: Option<String>,
    /// Token de dispositivo (Bearer) entregado por el backend al confirmar pairing.
    /// Se persiste en claro; el backend sólo guarda su SHA-256.
    #[serde(default)]
    pub device_token: Option<String>,
    #[serde(default = "default_device_id")]
    pub device_id: String,
    /// URL base del backend de Cegel. Configurable para entornos staging.
    #[serde(default = "default_api_base")]
    pub cegel_api_base: String,
    /// Impresoras configuradas localmente por el bridge.
    #[serde(default)]
    pub printers: Vec<PrinterConfig>,
}

/// Configuración de una impresora gestionada por el bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterConfig {
    pub id: String,
    pub name: String,
    /// Ancho del papel en mm: 80 o 58.
    #[serde(default = "default_paper_width")]
    pub paper_width_mm: u8,
    /// Pin del cajón monedero (0 = sin cajón, 2 o 5).
    #[serde(default)]
    pub drawer_pin: u8,
    /// Conexión detectada automáticamente por el bridge.
    pub connection: PrinterConnectionConfig,
    /// Si está Online según último test.
    #[serde(default)]
    pub online: bool,
}

/// Conexión detectada por el bridge (el usuario no la ingresa manualmente).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterConnectionConfig {
    #[serde(rename = "type")]
    pub conn_type: String, // "usb", "network", "serial", "winspool"
    pub host: Option<String>,
    pub port: Option<u16>,
    #[serde(rename = "vendorId")]
    pub vendor_id: Option<String>,
    #[serde(rename = "productId")]
    pub product_id: Option<String>,
    pub path: Option<String>,
}

fn default_paper_width() -> u8 {
    80
}

fn default_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_api_base() -> String {
    "https://api.cegel.app".to_string()
}

impl Default for BridgeConfig {
    fn default() -> Self {
        #[allow(unused_mut)]
        let mut allowed: Vec<String> = vec![
            "https://www.cegel.app".into(),
            "https://cegel.app".into(),
            "tauri://localhost".into(),
        ];
        // Orígenes de desarrollo solo en debug builds.
        #[cfg(debug_assertions)]
        {
            allowed.push("http://localhost:1420".into());
            allowed.push("http://127.0.0.1:1420".into());
            allowed.push("http://localhost:5173".into());
            allowed.push("http://127.0.0.1:5173".into());
        }
        Self {
            port: 9101,
            allowed_origins: allowed,
            paired_business_id: None,
            device_token: None,
            device_id: default_device_id(),
            cegel_api_base: default_api_base(),
            printers: Vec::new(),
        }
    }
}

impl BridgeConfig {
    /// Valida que la URL del backend sea HTTPS (excepto en debug builds).
    pub fn validate_api_base(&self) -> Result<()> {
        #[cfg(not(debug_assertions))]
        if !self.cegel_api_base.starts_with("https://") {
            return Err(anyhow!(
                "cegel_api_base debe usar HTTPS en producción (actual: {})",
                self.cegel_api_base
            ));
        }
        Ok(())
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::home_dir().context("no se pudo determinar HOME")?;
    let dir = base.join(".cegel");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("no se pudo crear ~/.cegel")?;
        // Permisos restrictivos: solo el usuario actual puede acceder.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }
    Ok(dir)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("bridge.json"))
}

pub fn load() -> BridgeConfig {
    match config_path().and_then(|p| {
        if !p.exists() {
            return Ok(BridgeConfig::default());
        }
        let raw = fs::read_to_string(&p)?;
        let cfg: BridgeConfig = serde_json::from_str(&raw)?;
        Ok(merge_with_defaults(cfg))
    }) {
        Ok(cfg) => cfg,
        Err(err) => {
            log::warn!("usando config por defecto ({err})");
            BridgeConfig::default()
        }
    }
}

fn merge_with_defaults(mut cfg: BridgeConfig) -> BridgeConfig {
    let defaults = BridgeConfig::default();
    for origin in defaults.allowed_origins {
        if !cfg
            .allowed_origins
            .iter()
            .any(|existing| existing == &origin)
        {
            cfg.allowed_origins.push(origin);
        }
    }
    cfg
}

pub fn save(cfg: &BridgeConfig) -> Result<()> {
    let path = config_path()?;
    let raw = serde_json::to_string_pretty(cfg)?;
    fs::write(&path, raw).context("no se pudo escribir bridge.json")?;
    // device_token es secreto: restringir permisos a 0600 (solo el dueño lee/escribe).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .context("no se pudo restringir permisos de bridge.json")?;
    }
    Ok(())
}

// ─── Printer management helpers ─────────────────────────────────────────────

impl BridgeConfig {
    pub fn find_printer(&self, id: &str) -> Option<&PrinterConfig> {
        self.printers.iter().find(|p| p.id == id)
    }

    pub fn find_printer_mut(&mut self, id: &str) -> Option<&mut PrinterConfig> {
        self.printers.iter_mut().find(|p| p.id == id)
    }

    pub fn add_printer(&mut self, mut printer: PrinterConfig) {
        if printer.id.is_empty() {
            printer.id = uuid::Uuid::new_v4().to_string();
        }
        self.printers.push(printer);
    }

    pub fn remove_printer(&mut self, id: &str) -> bool {
        let len_before = self.printers.len();
        self.printers.retain(|p| p.id != id);
        self.printers.len() < len_before
    }
}

/// Genera un ticket de prueba ESC/POS (80mm o 58mm).
pub fn generate_test_ticket(paper_width_mm: u8, open_drawer: bool) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();

    // Init
    buf.extend_from_slice(&[0x1B, 0x40]); // ESC @

    // Alineación centro
    buf.extend_from_slice(&[0x1B, 0x61, 0x01]);

    // Encabezado
    buf.extend_from_slice(b"\n");
    buf.extend_from_slice(b"==== CEGEL PRINT BRIDGE ====\n");
    buf.extend_from_slice(b"   Ticket de prueba\n");
    buf.extend_from_slice(b"\n");

    // Alineación izquierda
    buf.extend_from_slice(&[0x1B, 0x61, 0x00]);

    let line = if paper_width_mm <= 58 {
        "--------------------------------".as_bytes()
    } else {
        "------------------------------------------------".as_bytes()
    };
    buf.extend_from_slice(line);
    buf.extend_from_slice(b"\n");

    buf.extend_from_slice(b"Fecha : ");
    buf.extend_from_slice(chrono_now().as_bytes());
    buf.extend_from_slice(b"\n");
    buf.extend_from_slice(format!("Papel : {} mm\n", paper_width_mm).as_bytes());
    buf.extend_from_slice(b"Estado: Conexion exitosa\n");
    buf.extend_from_slice(line);
    buf.extend_from_slice(b"\n\n");

    // Centro
    buf.extend_from_slice(&[0x1B, 0x61, 0x01]);
    buf.extend_from_slice(b"TODO FUNCIONA CORRECTAMENTE\n");
    buf.extend_from_slice(b"La impresora esta lista para\n");
    buf.extend_from_slice(b"imprimir recibos y facturas.\n");
    buf.extend_from_slice(b"\n");

    buf.extend_from_slice(line);
    buf.extend_from_slice(b"\n");

    // Abrir cajón si se pidió
    if open_drawer {
        // Pulso pin 2 (común)
        buf.extend_from_slice(&[0x1B, 0x70, 0x00, 0x19, 0xFF]);
    }

    // Cortar
    buf.extend_from_slice(b"\n\n\n\n");
    buf.extend_from_slice(&[0x1D, 0x56, 0x00]); // GS V 0 (full cut)

    buf
}

fn chrono_now() -> String {
    // Fecha/hora simple sin depender de chrono crate
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Calcular fecha aproximada
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    // Fecha simple: 2024-01-01 + days
    let y = 2024 + (days / 365) as i64;
    let remaining = days % 365;
    let m = (remaining / 30) + 1;
    let d = (remaining % 30) + 1;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        y,
        m.clamp(1, 12),
        d.clamp(1, 28),
        hours,
        minutes
    )
}
