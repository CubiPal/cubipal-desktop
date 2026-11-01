mod calendar;
mod device;
mod tray;

use tauri::WindowEvent;

#[tauri::command]
fn show_preferences_window(app: tauri::AppHandle) {
    tray::show_preferences(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            tray::create_tray(app.handle())?;
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
            device::scan_for_pals,
            device::pair_pal,
            device::get_paired_device,
            device::unpair_pal,
            calendar::connect_google_account,
            calendar::disconnect_google_account,
            calendar::connect_microsoft_account,
            calendar::disconnect_microsoft_account,
            calendar::set_ics_url,
            calendar::disconnect_ics,
            calendar::get_connected_accounts,
            calendar::sync_calendar_events,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
