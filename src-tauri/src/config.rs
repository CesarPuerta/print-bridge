use anyhow::{Context, Result};
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
}

fn default_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_api_base() -> String {
    "https://api.cegel.app".to_string()
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            port: 9101,
            allowed_origins: vec![
                "https://www.cegel.app".into(),
                "https://cegel.app".into(),
                "http://localhost:5173".into(),
                "http://127.0.0.1:5173".into(),
            ],
            paired_business_id: None,
            device_token: None,
            device_id: default_device_id(),
            cegel_api_base: default_api_base(),
        }
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::home_dir().context("no se pudo determinar HOME")?;
    let dir = base.join(".cegel");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("no se pudo crear ~/.cegel")?;
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
        Ok(cfg)
    }) {
        Ok(cfg) => cfg,
        Err(err) => {
            log::warn!("usando config por defecto ({err})");
            BridgeConfig::default()
        }
    }
}

pub fn save(cfg: &BridgeConfig) -> Result<()> {
    let path = config_path()?;
    let raw = serde_json::to_string_pretty(cfg)?;
    fs::write(&path, raw).context("no se pudo escribir bridge.json")?;
    Ok(())
}
