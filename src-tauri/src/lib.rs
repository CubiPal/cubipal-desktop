mod calendar;
mod device;
mod ota;
mod settings;
mod tray;

use std::time::Duration;
use tauri::{Emitter, WindowEvent};
use tauri_plugin_updater::UpdaterExt;

#[tauri::command]
fn show_preferences_window(app: tauri::AppHandle) {
    tray::show_preferences(&app);
}

#[tauri::command]
async fn check_desktop_update(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(serde_json::json!({
            "version": update.version,
            "current_version": update.current_version,
            "body": update.body.unwrap_or_default(),
            "date": update.date.map(|d| d.to_string()).unwrap_or_default(),
        }))),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("Failed to check for updates: {}", e)),
    }
}

#[tauri::command]
async fn install_desktop_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
        update
            .download_and_install(|_chunk, _total| {}, || {})
            .await
            .map_err(|e| format!("Failed to download update: {}", e))?;

        // Safely restart application
        app.restart();
    }
    Ok(())
}

/// Checks for a new app version 30 s after startup and then every 24 h,
/// unless "Automatic Updates" is off (then nothing is fetched at all; the
/// tray's "Check for Updates..." still works).
fn start_update_checker(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Startup delay: wait 30s before first update check
        tokio::time::sleep(Duration::from_secs(30)).await;

        loop {
            if settings::load(&app).auto_update
                && let Ok(updater) = app.updater()
                && let Ok(Some(update)) = updater.check().await
            {
                let _ = app.emit(
                    "desktop-update-available",
                    serde_json::json!({
                        "version": update.version,
                        "current_version": update.current_version,
                        "body": update.body.unwrap_or_default(),
                    }),
                );
            }
            // Check once every 24 hours
            tokio::time::sleep(Duration::from_secs(86400)).await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            tray::create_tray(app.handle())?;
            calendar::start_background_sync(app.handle().clone());
            start_update_checker(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            show_preferences_window,
            check_desktop_update,
            install_desktop_update,
            settings::get_general_settings,
            settings::set_autostart,
            settings::set_auto_update,
            device::scan_for_pals,
            device::pair_pal,
            device::get_paired_device,
            device::unpair_pal,
            device::set_device_skin,
            device::start_device_timer,
            calendar::connect_google_account,
            calendar::disconnect_google_account,
            calendar::connect_microsoft_account,
            calendar::disconnect_microsoft_account,
            calendar::set_ics_url,
            calendar::disconnect_ics,
            calendar::get_connected_accounts,
            calendar::get_calendar_sync_config,
            calendar::save_calendar_sync_config,
            calendar::trigger_calendar_sync,
            calendar::sync_calendar_events,
            calendar::test_device_chime,
            ota::check_firmware_update,
            ota::install_firmware_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
