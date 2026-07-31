use crate::adapters;
use crate::adapters::usb_scan::UsbDeviceInfo;
use crate::config::{self, BridgeConfig, PrinterConfig, PrinterConnectionConfig};
use crate::types::{ErrorResponse, HealthResponse, JobResponse, PrintJob};

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use base64::{engine::general_purpose, Engine};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::cors::{AllowOrigin, CorsLayer};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

pub struct AppState {
    pub config: Mutex<BridgeConfig>,
}

pub fn build_router(config: BridgeConfig) -> Router {
    let allowed: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-cegel-business"),
        ])
        .allow_origin(AllowOrigin::list(allowed));

    let state = Arc::new(AppState {
        config: Mutex::new(config),
    });

    Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/print", post(print_job))
        .route("/drawer-kick", post(drawer_kick))
        .route("/usb-devices", get(usb_devices))
        .route("/usb/test", post(usb_test))
        .route("/printers", get(list_printers))
        .route("/printers/configure", post(configure_printer))
        .route("/printers/{id}", delete(delete_printer))
        .route("/printers/{id}/test", post(test_printer))
        .route("/printers/{id}/drawer", post(drawer_printer))
        .route("/printers/delete", post(delete_printer_body))
        .route("/printers/test", post(test_printer_body))
        .with_state(state)
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            MAX_BODY_BYTES,
        ))
        .layer(cors)
}

pub async fn run(config: BridgeConfig) -> anyhow::Result<()> {
    let port = config.port;
    let app = build_router(config.clone());
    let addr_v4 = format!("127.0.0.1:{port}");
    let addr_v6 = format!("[::1]:{port}");
    log::info!("Print Bridge escuchando en http://{addr_v4} y http://{addr_v6}");

    let listener_v4 = tokio::net::TcpListener::bind(&addr_v4).await?;
    let listener_v6 = tokio::net::TcpListener::bind(&addr_v6).await;

    if let Ok(listener_v6) = listener_v6 {
        log::info!("IPv6 loopback (::1) habilitado");
        let app_v6 = build_router(config);
        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener_v6, app_v6).await {
                log::error!("servidor IPv6 cayó: {err}");
            }
        });
    }

    axum::serve(listener_v4, app).await?;
    Ok(())
}

// Handlers

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let cfg = state.config.lock().await;
    Json(HealthResponse {
        ok: true,
        version: VERSION.to_string(),
        name: "Cegel Print Bridge",
        paired: cfg.paired_business_id.is_some(),
        business_id: cfg.paired_business_id.clone(),
        device_id: cfg.device_id.clone(),
    })
}

#[derive(serde::Serialize)]
struct UsbDevicesResponse {
    devices: Vec<UsbDeviceInfo>,
}

async fn usb_devices() -> Json<UsbDevicesResponse> {
    let devices = adapters::usb_scan::scan();
    Json(UsbDevicesResponse { devices })
}

#[derive(serde::Deserialize)]
struct UsbTestRequest {
    #[serde(rename = "vendorId")]
    vendor_id: String,
    #[serde(rename = "productId")]
    product_id: String,
}

async fn usb_test(Json(req): Json<UsbTestRequest>) -> Result<Json<JobResponse>, AppError> {
    let conn = crate::types::Connection {
        conn_type: crate::types::ConnectionType::Usb,
        host: None,
        port: None,
        vendor_id: Some(req.vendor_id),
        product_id: Some(req.product_id),
        path: None,
        baud_rate: None,
        mac_address: None,
    };

    let bytes = config::generate_test_ticket(80, false);
    adapters::send_bytes(&conn, &bytes).map_err(AppError::internal)?;

    Ok(Json(JobResponse {
        ok: true,
        job_id: uuid::Uuid::new_v4().to_string(),
        bytes: bytes.len(),
        message: Some("Ticket de prueba enviado".into()),
    }))
}

// Printer management

#[derive(serde::Serialize)]
struct PrinterListResponse {
    printers: Vec<PrinterInfo>,
}

#[derive(serde::Serialize, Clone)]
struct PrinterInfo {
    id: String,
    name: String,
    #[serde(rename = "paperWidthMm")]
    paper_width_mm: u8,
    #[serde(rename = "drawerPin")]
    drawer_pin: u8,
    online: bool,
    connection: PrinterConnectionInfo,
}

#[derive(serde::Serialize, Clone)]
struct PrinterConnectionInfo {
    #[serde(rename = "type")]
    conn_type: String,
    host: Option<String>,
    port: Option<u16>,
    #[serde(rename = "vendorId")]
    vendor_id: Option<String>,
    #[serde(rename = "productId")]
    product_id: Option<String>,
    path: Option<String>,
}

async fn list_printers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PrinterListResponse>, AppError> {
    // UI local del bridge — no requiere auth de negocio
    let cfg = state.config.lock().await;
    let printers: Vec<PrinterInfo> = cfg.printers.iter().map(printer_to_info).collect();
    Ok(Json(PrinterListResponse { printers }))
}

fn printer_to_info(p: &PrinterConfig) -> PrinterInfo {
    PrinterInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        paper_width_mm: p.paper_width_mm,
        drawer_pin: p.drawer_pin,
        online: p.online,
        connection: PrinterConnectionInfo {
            conn_type: p.connection.conn_type.clone(),
            host: p.connection.host.clone(),
            port: p.connection.port,
            vendor_id: p.connection.vendor_id.clone(),
            product_id: p.connection.product_id.clone(),
            path: p.connection.path.clone(),
        },
    }
}

#[derive(serde::Deserialize)]
struct ConfigurePrinterRequest {
    #[serde(default)]
    id: String,
    name: String,
    #[serde(default = "default_80", alias = "paperWidthMm")]
    paper_width_mm: u8,
    #[serde(default, alias = "drawerPin")]
    drawer_pin: u8,
    connection: Option<ConfigureConnectionRequest>,
}

fn default_80() -> u8 {
    80
}

#[derive(serde::Deserialize)]
struct ConfigureConnectionRequest {
    #[serde(rename = "type")]
    conn_type: String,
    host: Option<String>,
    port: Option<u16>,
    #[serde(rename = "vendorId")]
    vendor_id: Option<String>,
    #[serde(rename = "productId")]
    product_id: Option<String>,
    path: Option<String>,
}

async fn configure_printer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ConfigurePrinterRequest>,
) -> Result<Json<PrinterInfo>, AppError> {
    enforce_business(&state, &headers).await?;
    if req.name.trim().is_empty() {
        return Err(AppError::bad_request("name es requerido"));
    }

    let connection = if let Some(conn) = req.connection {
        PrinterConnectionConfig {
            conn_type: conn.conn_type,
            host: conn.host,
            port: conn.port,
            vendor_id: conn.vendor_id,
            product_id: conn.product_id,
            path: conn.path,
        }
    } else {
        let devices = adapters::usb_scan::scan();
        if let Some(d) = devices.first() {
            PrinterConnectionConfig {
                conn_type: "usb".into(),
                host: None,
                port: None,
                vendor_id: Some(d.vendor_id.clone()),
                product_id: Some(d.product_id.clone()),
                path: None,
            }
        } else {
            return Err(AppError::bad_request(
                "No se detecto impresora USB. Conectala o especifica connection.",
            ));
        }
    };

    let mut cfg = state.config.lock().await;

    let printer = if req.id.is_empty() {
        let id = uuid::Uuid::new_v4().to_string();
        let p = PrinterConfig {
            id: id.clone(),
            name: req.name.trim().to_string(),
            paper_width_mm: req.paper_width_mm,
            drawer_pin: req.drawer_pin,
            connection,
            online: false,
        };
        cfg.add_printer(p.clone());
        p
    } else {
        let p = cfg
            .find_printer_mut(&req.id)
            .ok_or_else(|| AppError::not_found("impresora no encontrada"))?;
        p.name = req.name.trim().to_string();
        p.paper_width_mm = req.paper_width_mm;
        p.drawer_pin = req.drawer_pin;
        p.connection = connection;
        p.clone()
    };

    if let Err(e) = config::save(&cfg) {
        log::error!("error guardando config: {e}");
    }

    Ok(Json(printer_to_info(&printer)))
}

async fn delete_printer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    log::info!("DELETE /printers/{} received", id);
    let mut cfg = state.config.lock().await;
    if !cfg.remove_printer(&id) {
        return Err(AppError::not_found("impresora no encontrada"));
    }
    if let Err(e) = config::save(&cfg) {
        log::error!("error guardando config: {e}");
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_printer_body(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = body["id"].as_str().unwrap_or("");
    log::info!("POST /printers/delete id={id}");
    let mut cfg = state.config.lock().await;
    if !cfg.remove_printer(id) {
        return Err(AppError::not_found("impresora no encontrada"));
    }
    if let Err(e) = config::save(&cfg) {
        log::error!("error guardando config: {e}");
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn test_printer_body(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<JobResponse>, AppError> {
    let id = body["id"].as_str().unwrap_or("");
    log::info!("POST /printers/test id={id}");
    let cfg = state.config.lock().await;
    let printer = cfg
        .find_printer(id)
        .ok_or_else(|| AppError::not_found("impresora no encontrada"))?;

    let bytes = config::generate_test_ticket(printer.paper_width_mm, printer.drawer_pin > 0);
    let connection = printer_connection_to_types(printer);
    drop(cfg);

    adapters::send_bytes(&connection, &bytes).map_err(AppError::internal)?;

    let mut cfg = state.config.lock().await;
    if let Some(p) = cfg.find_printer_mut(id) {
        p.online = true;
    }
    if let Err(e) = config::save(&cfg) {
        log::error!("error guardando config: {e}");
    }

    Ok(Json(JobResponse {
        ok: true,
        job_id: uuid::Uuid::new_v4().to_string(),
        bytes: bytes.len(),
        message: Some("Ticket de prueba enviado".into()),
    }))
}

async fn test_printer(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<JobResponse>, AppError> {
    // UI local del bridge — no requiere auth de negocio
    let cfg = state.config.lock().await;
    let printer = cfg
        .find_printer(&id)
        .ok_or_else(|| AppError::not_found("impresora no encontrada"))?;

    let bytes = config::generate_test_ticket(printer.paper_width_mm, printer.drawer_pin > 0);
    let connection = printer_connection_to_types(printer);

    adapters::send_bytes(&connection, &bytes).map_err(AppError::internal)?;

    drop(cfg);
    let mut cfg = state.config.lock().await;
    if let Some(p) = cfg.find_printer_mut(&id) {
        p.online = true;
    }
    if let Err(e) = config::save(&cfg) {
        log::error!("error guardando config: {e}");
    }

    Ok(Json(JobResponse {
        ok: true,
        job_id: uuid::Uuid::new_v4().to_string(),
        bytes: bytes.len(),
        message: Some("Ticket de prueba enviado".into()),
    }))
}

async fn drawer_printer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<JobResponse>, AppError> {
    enforce_business(&state, &headers).await?;
    let cfg = state.config.lock().await;
    let printer = cfg
        .find_printer(&id)
        .ok_or_else(|| AppError::not_found("impresora no encontrada"))?;

    if printer.drawer_pin == 0 {
        return Err(AppError::bad_request(
            "Esta impresora no tiene cajon configurado",
        ));
    }

    let pin_byte = if printer.drawer_pin == 5 { 0x01 } else { 0x00 };
    let bytes = vec![0x1B, 0x70, pin_byte, 0x19, 0xFF];

    let connection = printer_connection_to_types(printer);
    adapters::send_bytes(&connection, &bytes).map_err(AppError::internal)?;

    Ok(Json(JobResponse {
        ok: true,
        job_id: uuid::Uuid::new_v4().to_string(),
        bytes: bytes.len(),
        message: Some("Cajon abierto".into()),
    }))
}

// Print / Drawer kick

async fn print_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(job): Json<PrintJob>,
) -> Result<Json<JobResponse>, AppError> {
    enforce_business(&state, &headers).await?;
    process_print_job(&state, job, "print").await.map(Json)
}

async fn drawer_kick(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(job): Json<PrintJob>,
) -> Result<Json<JobResponse>, AppError> {
    enforce_business(&state, &headers).await?;
    process_print_job(&state, job, "drawer-kick")
        .await
        .map(Json)
}

async fn enforce_business(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let cfg = state.config.lock().await;
    let origin = headers
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    // Si no hay Origin, permitir solo desde localhost (CLI, backend, etc.).
    if origin.is_empty() {
        log::debug!("request sin Origin header — permitido (local)");
        return Ok(());
    }

    let origin_allowed = cfg.allowed_origins.iter().any(|o| o == origin);
    if !origin_allowed {
        log::warn!("Origin bloqueado: {origin}");
        return Err(AppError {
            status: StatusCode::FORBIDDEN,
            message: "Origin no autorizado".into(),
        });
    }
    Ok(())
}

async fn process_print_job(
    state: &AppState,
    job: PrintJob,
    kind: &str,
) -> Result<JobResponse, AppError> {
    if job.printer_id.is_empty() || job.printer_id.len() > 100 {
        return Err(AppError::bad_request("printerId invalido"));
    }
    if job.bytes_base64.len() > MAX_BODY_BYTES {
        return Err(AppError::bad_request("payload demasiado grande"));
    }

    let bytes = general_purpose::STANDARD
        .decode(job.bytes_base64.as_bytes())
        .map_err(|e| AppError::bad_request(format!("bytesBase64 invalido: {e}")))?;

    if bytes.is_empty() {
        return Err(AppError::bad_request("payload vacio"));
    }

    let connection = resolve_connection(state, &job).await?;
    adapters::send_bytes(&connection, &bytes).map_err(AppError::internal)?;

    let job_id = uuid::Uuid::new_v4().to_string();
    log::info!(
        "[{kind}] printerId={} bytes={} jobId={}",
        job.printer_id,
        bytes.len(),
        job_id
    );

    Ok(JobResponse {
        ok: true,
        job_id,
        bytes: bytes.len(),
        message: None,
    })
}

async fn resolve_connection(
    state: &AppState,
    job: &PrintJob,
) -> Result<crate::types::Connection, AppError> {
    let has_explicit = job.connection.vendor_id.is_some()
        || job.connection.host.is_some()
        || job.connection.path.is_some();

    if has_explicit {
        return Ok(job.connection.clone());
    }

    let cfg = state.config.lock().await;
    let printer = cfg
        .find_printer(&job.printer_id)
        .ok_or_else(|| AppError::not_found("impresora no encontrada en el bridge"))?;
    Ok(printer_connection_to_types(printer))
}

// Helpers

fn printer_connection_to_types(p: &PrinterConfig) -> crate::types::Connection {
    crate::types::Connection {
        conn_type: connection_type_from_str(&p.connection.conn_type),
        host: p.connection.host.clone(),
        port: p.connection.port,
        vendor_id: p.connection.vendor_id.clone(),
        product_id: p.connection.product_id.clone(),
        path: p.connection.path.clone(),
        baud_rate: None,
        mac_address: None,
    }
}

fn connection_type_from_str(s: &str) -> crate::types::ConnectionType {
    match s {
        "network" => crate::types::ConnectionType::Network,
        "usb" => crate::types::ConnectionType::Usb,
        "serial" => crate::types::ConnectionType::Serial,
        "bluetooth" => crate::types::ConnectionType::Bluetooth,
        #[cfg(windows)]
        "winspool" => crate::types::ConnectionType::Winspool,
        _ => crate::types::ConnectionType::Usb,
    }
}

// Error handling

pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(ErrorResponse {
            ok: false,
            error: self.message,
        });
        (self.status, body).into_response()
    }
}
