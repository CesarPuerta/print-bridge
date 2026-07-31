use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_autostart::MacosLauncher;

use crate::config::PrinterConfig;
use crate::config::PrinterConnectionConfig;

pub mod adapters;
pub mod config;
pub mod pairing;
pub mod server;
pub mod types;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = config::load();
    let pairing_state = pairing::new_state();

    // 1) Servidor HTTP local en una tarea Tokio dedicada.
    let server_cfg = cfg.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("no se pudo crear runtime Tokio");
        rt.block_on(async move {
            if let Err(err) = server::run(server_cfg).await {
                log::error!("servidor HTTP cayó: {err}");
            }
        });
    });

    // 2) Loop de heartbeat al backend de Cegel.
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().expect("no se pudo crear runtime Tokio (hb)");
        rt.block_on(async move {
            pairing::run_heartbeat().await;
        });
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(pairing_state)
        .invoke_handler(tauri::generate_handler![
            cmd_get_config,
            cmd_save_config,
            cmd_get_status,
            cmd_check_health,
            cmd_start_pairing,
            cmd_get_pairing_state,
            cmd_unpair,
            cmd_set_autostart,
            cmd_scan_usb,
            cmd_test_usb,
            cmd_list_printers,
            cmd_configure_printer,
        ])
        .setup(move |app| {
            // Habilitar autostart por defecto la primera vez.
            {
                use tauri_plugin_autostart::ManagerExt;
                let manager = app.autolaunch();
                if !manager.is_enabled().unwrap_or(false) {
                    let _ = manager.enable();
                }
            }

            // Tray con menú básico.
            let show = MenuItem::with_id(app, "show", "Mostrar ventana", true, None::<&str>)?;
            let pair = MenuItem::with_id(app, "pair", "Vincular equipo…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &pair, &quit])?;

            let _tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .tooltip(format!("Cegel Print Bridge :{}", cfg.port))
                .icon(app.default_window_icon().unwrap().clone())
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "pair" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.eval("window.location.hash = '#/pair'");
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error iniciando Cegel Print Bridge");
}

// ─── Comandos expuestos al frontend ────────────────────────────────────────

#[derive(serde::Serialize)]
struct SafeConfig {
    port: u16,
    allowed_origins: Vec<String>,
    paired_business_id: Option<String>,
    device_id: String,
    cegel_api_base: String,
}

#[tauri::command]
fn cmd_get_config() -> Result<SafeConfig, String> {
    let cfg = config::load();
    Ok(SafeConfig {
        port: cfg.port,
        allowed_origins: cfg.allowed_origins,
        paired_business_id: cfg.paired_business_id,
        device_id: cfg.device_id,
        cegel_api_base: cfg.cegel_api_base,
    })
}

#[tauri::command]
fn cmd_save_config(cfg: config::BridgeConfig) -> Result<(), String> {
    config::save(&cfg).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct Status {
    ok: bool,
    version: String,
    port: u16,
    paired: bool,
    device_id: String,
    business_id: Option<String>,
}

#[tauri::command]
fn cmd_get_status() -> Status {
    let cfg = config::load();
    Status {
        ok: true,
        version: env!("CARGO_PKG_VERSION").into(),
        port: cfg.port,
        paired: cfg.device_token.is_some(),
        device_id: cfg.device_id,
        business_id: cfg.paired_business_id,
    }
}

/// Health check del servidor HTTP local — se ejecuta desde Rust para evitar
/// restricciones de red de WebView2 en Windows (que bloquea fetch() a loopback).
#[tauri::command]
async fn cmd_check_health() -> Result<bool, String> {
    let cfg = config::load();
    let url = format!("http://127.0.0.1:{}/health", cfg.port);
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
async fn cmd_start_pairing(state: tauri::State<'_, pairing::SharedState>) -> Result<(), String> {
    let shared = state.inner().clone();
    tokio::spawn(async move {
        if let Err(e) = pairing::run_pairing(shared).await {
            log::error!("pairing terminó con error: {e}");
        }
    });
    Ok(())
}
#[tauri::command]
fn cmd_get_pairing_state(
    state: tauri::State<'_, pairing::SharedState>,
) -> Result<pairing::PairingState, String> {
    state
        .read()
        .map(|s| s.clone())
        .map_err(|e| format!("estado bloqueado: {e}"))
}

#[tauri::command]
async fn cmd_unpair() -> Result<(), String> {
    let cfg = config::load();

    // 1. Intentar revocar en el backend (mejor esfuerzo).
    if let Some(token) = &cfg.device_token {
        let url = format!("{}/api/devices/unpair", cfg.cegel_api_base);
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .use_rustls_tls()
            .build()
        {
            Ok(c) => c,
            Err(_) => {
                log::warn!("no se pudo crear cliente HTTP para unpair");
                return Err("error de red".into());
            }
        };
        match client.post(&url).bearer_auth(token).send().await {
            Ok(r) if r.status().is_success() => log::info!("dispositivo revocado en backend"),
            Ok(r) => log::warn!("unpair HTTP {}", r.status()),
            Err(e) => log::warn!("unpair error: {e}"),
        }
    }

    // 2. Limpiar estado local.
    let defaults = config::BridgeConfig::default();
    let mut cfg = config::load();
    cfg.device_token = None;
    cfg.paired_business_id = None;
    cfg.printers.clear();
    cfg.device_id = defaults.device_id;
    config::save(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}

/// Escanea impresoras USB conectadas — evita fetch() a localhost bloqueado en Windows.
#[tauri::command]
fn cmd_scan_usb() -> Result<Vec<adapters::usb_scan::UsbDeviceInfo>, String> {
    Ok(adapters::usb_scan::scan())
}

/// Envía ticket de prueba a un dispositivo USB — evita fetch() a localhost.
#[tauri::command]
fn cmd_test_usb(vendor_id: String, product_id: String) -> Result<bool, String> {
    let connection = types::Connection {
        conn_type: types::ConnectionType::Usb,
        vendor_id: Some(vendor_id),
        product_id: Some(product_id),
        host: None,
        port: None,
        path: None,
        baud_rate: None,
        mac_address: None,
    };
    let test_ticket = config::generate_test_ticket(80, false);
    adapters::send_bytes(&connection, &test_ticket).map_err(|e| format!("{e}"))?;
    Ok(true)
}

/// Lista impresoras configuradas — evita fetch() a localhost.
#[tauri::command]
fn cmd_list_printers() -> Result<serde_json::Value, String> {
    let cfg = config::load();
    let printers: Vec<serde_json::Value> = cfg
        .printers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "paperWidthMm": p.paper_width_mm,
                "drawerPin": p.drawer_pin,
                "online": p.online,
                "connection": {
                    "type": p.connection.conn_type,
                    "vendorId": p.connection.vendor_id,
                    "productId": p.connection.product_id,
                }
            })
        })
        .collect();
    Ok(serde_json::json!({ "printers": printers }))
}

/// Configura una impresora (crea o actualiza) — evita fetch() a localhost en Windows.
#[tauri::command(rename_all = "camelCase")]
fn cmd_configure_printer(
    id: Option<String>,
    name: String,
    paper_width_mm: Option<u8>,
    drawer_pin: Option<u8>,
    vendor_id: Option<String>,
    product_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut cfg = config::load();
    let connection = if let (Some(vid), Some(pid)) = (&vendor_id, &product_id) {
        PrinterConnectionConfig {
            conn_type: "usb".into(),
            host: None,
            port: None,
            vendor_id: Some(vid.clone()),
            product_id: Some(pid.clone()),
            path: None,
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
            return Err("No se detectó impresora USB".into());
        }
    };

    let name = name.trim().to_string();
    let pw = paper_width_mm.unwrap_or(80);
    let dp = drawer_pin.unwrap_or(0);

    let printer = if let Some(ref pid) = id {
        if let Some(p) = cfg.find_printer_mut(pid) {
            p.name = name;
            p.paper_width_mm = pw;
            p.drawer_pin = dp;
            p.connection = connection;
            p.clone()
        } else {
            return Err("Impresora no encontrada".into());
        }
    } else {
        let new_id = uuid::Uuid::new_v4().to_string();
        let p = PrinterConfig {
            id: new_id.clone(),
            name,
            paper_width_mm: pw,
            drawer_pin: dp,
            connection,
            online: false,
        };
        cfg.add_printer(p.clone());
        p
    };

    config::save(&cfg).map_err(|e| format!("Error guardando: {e}"))?;

    Ok(serde_json::json!({
        "id": printer.id,
        "name": printer.name,
        "paperWidthMm": printer.paper_width_mm,
        "drawerPin": printer.drawer_pin,
        "online": printer.online,
    }))
}
