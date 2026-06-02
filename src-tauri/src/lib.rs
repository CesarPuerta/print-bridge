use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_autostart::MacosLauncher;

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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(pairing_state)
        .invoke_handler(tauri::generate_handler![
            cmd_get_config,
            cmd_save_config,
            cmd_get_status,
            cmd_start_pairing,
            cmd_get_pairing_state,
            cmd_unpair,
            cmd_set_autostart,
            cmd_check_update,
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

            // Chequeo de actualización al arrancar (silencioso, fire-and-forget).
            // Si encuentra una versión nueva, el plugin updater muestra el
            // diálogo nativo (configurado en tauri.conf.json → dialog: true).
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri_plugin_updater::UpdaterExt;
                    match handle.updater() {
                        Ok(updater) => match updater.check().await {
                            Ok(Some(update)) => {
                                log::info!(
                                    "actualización disponible: {} → instalando…",
                                    update.version
                                );
                                if let Err(e) =
                                    update.download_and_install(|_, _| {}, || {}).await
                                {
                                    log::warn!("falló auto-update: {e}");
                                }
                            }
                            Ok(None) => log::info!("bridge al día"),
                            Err(e) => log::warn!("chequeo de update falló: {e}"),
                        },
                        Err(e) => log::warn!("updater no disponible: {e}"),
                    }
                });
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


#[derive(serde::Serialize)]
struct UpdateInfo {
    available: bool,
    version: Option<String>,
    current: String,
}

/// Chequeo manual de actualizaciones desde el frontend.
/// Si encuentra una versión nueva, la descarga e instala.
#[tauri::command]
async fn cmd_check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;
    let current = env!("CARGO_PKG_VERSION").to_string();
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|e| e.to_string())?;
            Ok(UpdateInfo {
                available: true,
                version: Some(version),
                current,
            })
        }
        Ok(None) => Ok(UpdateInfo {
            available: false,
            version: None,
            current,
        }),
        Err(e) => Err(e.to_string()),
    }
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
fn cmd_unpair() -> Result<(), String> {
    let mut cfg = config::load();
    cfg.device_token = None;
    cfg.paired_business_id = None;
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
