use crate::adapters;
use crate::config::BridgeConfig;
use crate::types::{ErrorResponse, HealthResponse, JobResponse, PrintJob};

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use base64::{engine::general_purpose, Engine};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Límite duro al tamaño del cuerpo de un job (base64 incluido). 5 MB cubre
/// imágenes ESC/POS razonables y evita DoS por payloads enormes.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

pub struct AppState {
    pub config: BridgeConfig,
}

pub fn build_router(config: BridgeConfig) -> Router {
    let allowed: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-cegel-business"),
        ])
        .allow_origin(AllowOrigin::list(allowed));

    let state = Arc::new(AppState { config });

    Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/print", post(print_job))
        .route("/drawer-kick", post(drawer_kick))
        .with_state(state)
        .layer(tower_http::limit::RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(cors)
}

pub async fn run(config: BridgeConfig) -> anyhow::Result<()> {
    let port = config.port;
    let app = build_router(config);
    let addr = format!("127.0.0.1:{port}");
    log::info!("Print Bridge escuchando en http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ─── Handlers ───────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: VERSION.to_string(),
        name: "Cegel Print Bridge",
        paired: state.config.paired_business_id.is_some(),
        business_id: state.config.paired_business_id.clone(),
        device_id: state.config.device_id.clone(),
    })
}

async fn print_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(job): Json<PrintJob>,
) -> Result<Json<JobResponse>, AppError> {
    enforce_business(&state, &headers)?;
    process_job(job, "print").map(Json)
}

async fn drawer_kick(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(job): Json<PrintJob>,
) -> Result<Json<JobResponse>, AppError> {
    enforce_business(&state, &headers)?;
    process_job(job, "drawer-kick").map(Json)
}

/// Valida que el llamante sea legítimo:
///   1. Origin siempre debe estar en la allowlist (defensa adicional frente a CSRF
///      desde sitios maliciosos que no envían preflight — fetch simple no requiere CORS
///      pero sí envía Origin).
///   2. Si el bridge está pareado, el header X-Cegel-Business debe coincidir.
fn enforce_business(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    let origin = headers
        .get("origin")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let origin_allowed = !origin.is_empty()
        && state.config.allowed_origins.iter().any(|o| o == origin);
    if !origin_allowed {
        return Err(AppError {
            status: StatusCode::FORBIDDEN,
            message: "Origin no autorizado".into(),
        });
    }

    let Some(paired_biz) = state.config.paired_business_id.as_deref() else {
        return Ok(()); // bridge sin vincular: permite (modo prueba)
    };
    let provided = headers
        .get("x-cegel-business")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if provided == paired_biz {
        Ok(())
    } else {
        Err(AppError {
            status: StatusCode::FORBIDDEN,
            message: format!(
                "Este equipo está vinculado a otro negocio (esperado: {paired_biz})"
            ),
        })
    }
}

fn process_job(job: PrintJob, kind: &str) -> Result<JobResponse, AppError> {
    // Validar estructura básica antes de decodificar payloads grandes.
    if job.printer_id.is_empty() || job.printer_id.len() > 100 {
        return Err(AppError::bad_request("printerId inválido"));
    }
    if let Some(l) = &job.label {
        if l.len() > 500 {
            return Err(AppError::bad_request("label demasiado largo"));
        }
    }
    if job.bytes_base64.len() > MAX_BODY_BYTES {
        return Err(AppError::bad_request("payload demasiado grande"));
    }

    let bytes = general_purpose::STANDARD
        .decode(job.bytes_base64.as_bytes())
        .map_err(|e| AppError::bad_request(format!("bytesBase64 inválido: {e}")))?;

    if bytes.is_empty() {
        return Err(AppError::bad_request("payload vacío"));
    }

    adapters::send_bytes(&job.connection, &bytes).map_err(AppError::internal)?;

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

// ─── Error handling ─────────────────────────────────────────────────────────

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
