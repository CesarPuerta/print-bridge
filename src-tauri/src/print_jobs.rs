//! Cola de impresión remota.
//!
//! El bridge escucha en loopback, así que un celular no puede enviarle bytes directo
//! (además el navegador bloquea HTTP desde una página HTTPS). La web deja el tiquete en
//! `/api/print-jobs`, el servidor le adjunta los bytes ESC/POS y este worker los recoge con
//! el token del dispositivo para escribirlos en la impresora física del local.

use crate::adapters;
use crate::config::{self, BridgeConfig};
use crate::server::printer_connection_to_types;
use crate::types::Connection;
use base64::{engine::general_purpose, Engine};
use serde::Deserialize;
use std::time::Duration;

/// Latido de la cola: 3 s mantiene la impresión «inmediata» sin inundar el API.
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const ACK_ATTEMPTS: u32 = 3;

#[derive(Deserialize)]
struct Envelope<T> {
    success: bool,
    data: Option<T>,
}

#[derive(Deserialize)]
struct RemoteJob {
    id: String,
    #[serde(rename = "printerId")]
    printer_id: String,
    connection: Connection,
    #[serde(rename = "bytesBase64")]
    bytes_base64: String,
    label: Option<String>,
}

fn api_url(cfg: &BridgeConfig, path: &str) -> String {
    format!("{}{}", cfg.cegel_api_base.trim_end_matches('/'), path)
}

/// Loop infinito: se lanza en su propio hilo al arrancar la app.
pub async fn run_worker() {
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if let Err(err) = poll_once().await {
            // Sin red o API caído es normal en una caja: se reintenta al siguiente ciclo.
            log::debug!("cola de impresión: {err}");
        }
    }
}

async fn poll_once() -> anyhow::Result<()> {
    let cfg = config::load();
    let Some(token) = cfg.device_token.clone() else {
        return Ok(());
    };

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .use_rustls_tls()
        .build()?;

    let res = client
        .get(api_url(&cfg, "/api/devices/print-jobs"))
        .bearer_auth(&token)
        .send()
        .await?;

    if !res.status().is_success() {
        log::warn!("print-jobs HTTP {}", res.status());
        return Ok(());
    }

    let body: Envelope<serde_json::Value> = res.json().await?;
    if !body.success {
        return Ok(());
    }
    let Some(payload) = body.data else {
        return Ok(());
    };
    let Some(jobs) = payload.get("jobs").and_then(|value| value.as_array()) else {
        return Ok(());
    };

    for raw in jobs {
        // Un trabajo con forma inválida no debe tumbar el resto del lote: se confirma como
        // fallido y se sigue (si no, el servidor lo reintentaría para siempre).
        match serde_json::from_value::<RemoteJob>(raw.clone()) {
            Ok(job) => {
                let outcome = print_job(&cfg, &job);
                match &outcome {
                    Ok(()) => log::info!(
                        "impreso {} ({})",
                        job.label.as_deref().unwrap_or("job"),
                        job.id
                    ),
                    Err(err) => log::warn!(
                        "falló {} ({}): {err}",
                        job.label.as_deref().unwrap_or("job"),
                        job.id
                    ),
                }
                ack(&client, &cfg, &token, &job.id, outcome).await;
            }
            Err(err) => {
                log::warn!("trabajo de impresión ilegible: {err}");
                if let Some(id) = raw.get("id").and_then(|value| value.as_str()) {
                    let detail = format!("Trabajo de impresión ilegible: {err}");
                    ack(&client, &cfg, &token, id, Err(detail)).await;
                }
            }
        }
    }

    Ok(())
}

/// Imprime un trabajo. La conexión viaja en el propio job (la conoce el servidor), así que
/// la impresora no tiene que estar «configurada» en este bridge.
fn print_job(cfg: &BridgeConfig, job: &RemoteJob) -> Result<(), String> {
    let bytes = general_purpose::STANDARD
        .decode(job.bytes_base64.as_bytes())
        .map_err(|e| format!("bytesBase64 inválido: {e}"))?;
    if bytes.is_empty() {
        return Err("payload vacío".into());
    }

    let connection = resolve_connection(cfg, job);
    adapters::send_bytes(&connection, &bytes).map_err(|e| e.to_string())
}

fn resolve_connection(cfg: &BridgeConfig, job: &RemoteJob) -> Connection {
    let has_explicit = job.connection.vendor_id.is_some()
        || job.connection.host.is_some()
        || job.connection.path.is_some();

    if has_explicit {
        return job.connection.clone();
    }

    match cfg.find_printer(&job.printer_id) {
        Some(printer) => printer_connection_to_types(printer),
        // Sin datos de conexión y sin impresora local: se intenta USB genérico para que el
        // error que llegue al ack sea el del adaptador y no un «no encontrada» opaco.
        None => job.connection.clone(),
    }
}

async fn ack(
    client: &reqwest::Client,
    cfg: &BridgeConfig,
    token: &str,
    job_id: &str,
    outcome: Result<(), String>,
) {
    let body = match outcome {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(err) => serde_json::json!({
            "ok": false,
            "error": err.chars().take(400).collect::<String>(),
        }),
    };
    let url = api_url(cfg, &format!("/api/devices/print-jobs/{job_id}/ack"));

    for attempt in 1..=ACK_ATTEMPTS {
        match client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => return,
            Ok(res) => log::warn!("ack HTTP {} (intento {attempt})", res.status()),
            Err(err) => log::warn!("ack error: {err} (intento {attempt})"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    log::error!("no se pudo confirmar el trabajo {job_id}; el servidor lo reintentará");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Payload REAL de `GET /api/devices/print-jobs` (capturado contra la app completa).
    /// Fija el contrato con el API: si cambia un nombre de campo, este test lo delata en vez
    /// de descubrirlo con la impresora del local sin imprimir.
    const CLAIM_PAYLOAD: &str = r#"{
      "success": true,
      "message": "1 trabajo(s) de impresión",
      "data": {
        "jobs": [
          {
            "id": "6adfb50544911198a73b8960",
            "printerId": "6adfb50544911198a73b895a",
            "connection": { "type": "usb", "vendorId": "0471", "productId": "0055" },
            "bytesBase64": "G0AbRRt0Ew==",
            "label": "invoice:6adfb50544911198a73b895f"
          }
        ]
      }
    }"#;

    #[test]
    fn parsea_la_cola_tal_como_la_manda_el_api() {
        let body: Envelope<serde_json::Value> = serde_json::from_str(CLAIM_PAYLOAD).unwrap();
        assert!(body.success);

        let jobs = body
            .data
            .unwrap()
            .get("jobs")
            .and_then(|value| value.as_array())
            .unwrap()
            .clone();
        assert_eq!(jobs.len(), 1);

        let job: RemoteJob = serde_json::from_value(jobs[0].clone()).unwrap();
        assert_eq!(job.id, "6adfb50544911198a73b8960");
        assert_eq!(job.printer_id, "6adfb50544911198a73b895a");
        assert_eq!(
            job.label.as_deref(),
            Some("invoice:6adfb50544911198a73b895f")
        );
        assert!(matches!(
            job.connection.conn_type,
            crate::types::ConnectionType::Usb
        ));
        assert_eq!(job.connection.vendor_id.as_deref(), Some("0471"));
        assert_eq!(job.connection.product_id.as_deref(), Some("0055"));
        assert!(!general_purpose::STANDARD
            .decode(job.bytes_base64.as_bytes())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn un_job_ilegible_no_arrastra_al_resto_del_lote() {
        let raw = serde_json::json!({ "id": "job-1", "printerId": "p1" });
        assert!(serde_json::from_value::<RemoteJob>(raw).is_err());
    }
}
