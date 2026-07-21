use serde::{Deserialize, Serialize};

/// Tipo de conexión física a la impresora.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    Network,
    Usb,
    Serial,
    Bluetooth,
}

/// Descripción de la conexión que envía la app web junto con cada trabajo.
/// Los campos son opcionales porque dependen del tipo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    #[serde(rename = "type")]
    pub conn_type: ConnectionType,

    // network
    pub host: Option<String>,
    pub port: Option<u16>,

    // usb (hex strings, sin "0x")
    #[serde(rename = "vendorId")]
    pub vendor_id: Option<String>,
    #[serde(rename = "productId")]
    pub product_id: Option<String>,

    // serial
    pub path: Option<String>,
    #[serde(rename = "baudRate")]
    pub baud_rate: Option<u32>,

    // bluetooth
    #[serde(rename = "macAddress")]
    pub mac_address: Option<String>,
}

/// Job recibido por POST /print o /drawer-kick.
#[derive(Debug, Clone, Deserialize)]
pub struct PrintJob {
    #[serde(rename = "bytesBase64")]
    pub bytes_base64: String,
    pub connection: Connection,
    #[serde(rename = "printerId")]
    pub printer_id: String,
    pub label: Option<String>,
}

/// Respuesta uniforme del bridge.
#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub ok: bool,
    #[serde(rename = "jobId")]
    pub job_id: String,
    pub bytes: usize,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub version: String,
    pub name: &'static str,
    #[serde(rename = "paired")]
    pub paired: bool,
    #[serde(rename = "businessId")]
    pub business_id: Option<String>,
    #[serde(rename = "deviceId")]
    pub device_id: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub error: String,
}
