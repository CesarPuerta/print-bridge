use crate::config::{self, BridgeConfig};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const POLL_TIMEOUT: Duration = Duration::from_secs(8);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Estado del proceso de pairing — expuesto a la UI vía Tauri commands.
#[derive(Debug, Clone, Serialize)]
pub struct PairingState {
    pub status: String, // "idle" | "waiting" | "paired" | "error"
    pub pairing_code: Option<String>,
    pub expires_at: Option<String>,
    pub error: Option<String>,
}

impl Default for PairingState {
    fn default() -> Self {
        Self {
            status: "idle".into(),
            pairing_code: None,
            expires_at: None,
            error: None,
        }
    }
}

#[derive(Deserialize)]
struct StartResp {
    success: bool,
    data: Option<StartData>,
    message: Option<String>,
}
#[derive(Deserialize)]
struct StartData {
    #[serde(rename = "pairingCode")]
    pairing_code: String,
    #[serde(rename = "expiresAt")]
    expires_at: String,
}

#[derive(Deserialize)]
struct PollResp {
    success: bool,
    data: Option<PollData>,
    message: Option<String>,
}
#[derive(Deserialize)]
struct PollData {
    status: String,
    token: Option<String>,
    #[serde(rename = "businessId")]
    business_id: Option<String>,
}

#[derive(Serialize)]
struct StartBody<'a> {
    #[serde(rename = "deviceId")]
    device_id: &'a str,
    name: String,
    os: String,
    hostname: String,
    #[serde(rename = "bridgeVersion")]
    bridge_version: &'a str,
}

fn os_label() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

fn host_label() -> String {
    gethostname::gethostname().to_string_lossy().to_string()
}

/// Estado compartido que la UI puede consultar.
pub type SharedState = Arc<RwLock<PairingState>>;

pub fn new_state() -> SharedState {
    Arc::new(RwLock::new(PairingState::default()))
}

/// Inicia el flujo de pairing en background: pide un código al backend,
/// lo muestra en `state` y entra en loop de polling hasta que el cajero
/// claime el código en la web. Cuando se confirma, persiste el token.
pub async fn run_pairing(state: SharedState) -> Result<()> {
    let cfg = config::load();
    cfg.validate_api_base().context("backend URL inválida")?;
    let client = reqwest::Client::builder()
        .timeout(POLL_TIMEOUT)
        .use_rustls_tls()
        .build()
        .context("no se pudo crear cliente HTTP")?;

    // 1. start-pairing
    let body = StartBody {
        device_id: &cfg.device_id,
        name: format!("Equipo {}", &cfg.device_id[..8]),
        os: os_label(),
        hostname: host_label(),
        bridge_version: VERSION,
    };
    let url = format!("{}/api/devices/start-pairing", cfg.cegel_api_base);
    let resp: StartResp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("error contactando backend (start-pairing)")?
        .json()
        .await
        .context("respuesta de start-pairing no parseable")?;

    if !resp.success {
        let msg = resp.message.unwrap_or_else(|| "start-pairing falló".into());
        write_state(&state, PairingState {
            status: "error".into(),
            pairing_code: None,
            expires_at: None,
            error: Some(msg.clone()),
        });
        return Err(anyhow!(msg));
    }
    let data = resp.data.ok_or_else(|| anyhow!("payload start-pairing vacío"))?;

    write_state(&state, PairingState {
        status: "waiting".into(),
        pairing_code: Some(data.pairing_code.clone()),
        expires_at: Some(data.expires_at.clone()),
        error: None,
    });

    // 2. Polling hasta paired / revoked / timeout
    let poll_url = format!(
        "{}/api/devices/poll-pairing/{}?code={}",
        cfg.cegel_api_base, cfg.device_id, data.pairing_code
    );
    let started = std::time::Instant::now();
    let max_wait = Duration::from_secs(10 * 60);

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if started.elapsed() > max_wait {
            write_state(&state, PairingState {
                status: "error".into(),
                pairing_code: None,
                expires_at: None,
                error: Some("Código expirado sin vincular".into()),
            });
            return Err(anyhow!("pairing timeout"));
        }

        let r: PollResp = match client.get(&poll_url).send().await {
            Ok(r) => match r.json().await {
                Ok(j) => j,
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        if !r.success {
            continue;
        }
        let Some(d) = r.data else { continue };

        match d.status.as_str() {
            "pending" => continue,
            "paired" => {
                let token = d.token.ok_or_else(|| anyhow!("paired sin token"))?;
                let mut new_cfg = cfg.clone();
                new_cfg.device_token = Some(token);
                new_cfg.paired_business_id = d.business_id;
                config::save(&new_cfg).context("no se pudo persistir bridge.json")?;
                write_state(&state, PairingState {
                    status: "paired".into(),
                    pairing_code: None,
                    expires_at: None,
                    error: None,
                });
                return Ok(());
            }
            "revoked" => {
                write_state(&state, PairingState {
                    status: "error".into(),
                    pairing_code: None,
                    expires_at: None,
                    error: Some("Dispositivo revocado por el negocio".into()),
                });
                return Err(anyhow!("revoked"));
            }
            other => {
                log::warn!("estado de pairing desconocido: {other}");
                continue;
            }
        }
    }
}

fn write_state(state: &SharedState, value: PairingState) {
    if let Ok(mut w) = state.write() {
        *w = value;
    }
}

/// Loop de heartbeat: cada 60s envía POST /api/devices/heartbeat con el token.
pub async fn run_heartbeat() {
    loop {
        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
        let cfg = config::load();
        let Some(token) = cfg.device_token.clone() else {
            continue;
        };
        if cfg.validate_api_base().is_err() {
            log::error!("heartbeat omitido: cegel_api_base debe ser HTTPS en producción");
            continue;
        }
        let client = match reqwest::Client::builder()
            .timeout(POLL_TIMEOUT)
            .use_rustls_tls()
            .build()
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        let url = format!("{}/api/devices/heartbeat", cfg.cegel_api_base);
        let body = serde_json::json!({ "bridgeVersion": VERSION });
        match client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                log::debug!("heartbeat OK");
            }
            Ok(r) => log::warn!("heartbeat HTTP {}", r.status()),
            Err(e) => log::warn!("heartbeat error: {e}"),
        }
    }
}

pub fn build_pairing_state() -> BridgeConfig {
    config::load()
}
