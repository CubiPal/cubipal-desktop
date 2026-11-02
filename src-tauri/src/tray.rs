use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub struct TrayMenuItems {
    pub status: MenuItem<tauri::Wry>,
    pub next_event: MenuItem<tauri::Wry>,
}

pub static TRAY_ITEMS: Mutex<Option<TrayMenuItems>> = Mutex::new(None);

pub fn update_tray_status(text: &str) {
    if let Ok(guard) = TRAY_ITEMS.lock()
        && let Some(items) = guard.as_ref()
    {
        let _ = items.status.set_text(text);
    }
}

pub fn update_tray_next_event(text: &str) {
    if let Ok(guard) = TRAY_ITEMS.lock()
        && let Some(items) = guard.as_ref()
    {
        let _ = items.next_event.set_text(text);
    }
}

pub fn create_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let status_item =
        MenuItem::with_id(app, "status", "⚪ CubiPal — Offline", false, None::<&str>)?;
    let next_event_item = MenuItem::with_id(
        app,
        "next_event",
        "Next: No upcoming events",
        false,
        None::<&str>,
    )?;

    if let Ok(mut guard) = TRAY_ITEMS.lock() {
        *guard = Some(TrayMenuItems {
            status: status_item.clone(),
            next_event: next_event_item.clone(),
        });
    }

    // Focus Timer Submenu (presets: 30m, 60m, custom, cancel)
    let timer_30 = MenuItem::with_id(app, "timer_30", "30 min Focus", true, None::<&str>)?;
    let timer_60 = MenuItem::with_id(app, "timer_60", "60 min Deep Work", true, None::<&str>)?;
    let timer_custom =
        MenuItem::with_id(app, "timer_custom", "Custom Focus...", true, None::<&str>)?;
    let timer_cancel = MenuItem::with_id(
        app,
        "timer_cancel",
        "Cancel Active Timer",
        true,
        None::<&str>,
    )?;

    let timer_sub = Submenu::with_items(
        app,
        "Focus Timer",
        true,
        &[&timer_30, &timer_60, &timer_custom, &timer_cancel],
    )?;

    let snooze_item =
        MenuItem::with_id(app, "snooze", "Snooze Alerts (1 hour)", true, None::<&str>)?;
    let sync_item = MenuItem::with_id(app, "sync_now", "Sync Calendars Now", true, None::<&str>)?;

    let open_web_item = MenuItem::with_id(
        app,
        "open_web",
        "Open Device Web Control",
        true,
        None::<&str>,
    )?;
    let prefs_item = MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>)?;
    let updates_item = MenuItem::with_id(
        app,
        "check_updates",
        "Check for Updates...",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit CubiPal", true, None::<&str>)?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let sep4 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &next_event_item,
            &sep1,
            &timer_sub,
            &sep2,
            &snooze_item,
            &sync_item,
            &sep3,
            &open_web_item,
            &prefs_item,
            &updates_item,
            &sep4,
            &quit_item,
        ],
    )?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("CubiPal Desktop Companion")
        .icon(app.default_window_icon().unwrap().clone())
        .on_menu_event(|app, event| match event.id.as_ref() {
            "preferences" => {
                show_preferences(app);
            }
            "open_web" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("open-device-web", ());
                }
            }
            "check_updates" => {
                show_preferences(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("check-desktop-updates", ());
                }
            }
            "sync_now" => {
                crate::calendar::trigger_immediate_sync();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("sync-now", ());
                }
            }
            "timer_30" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("start-timer", 30);
                }
            }
            "timer_60" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("start-timer", 60);
                }
            }
            "timer_custom" => {
                show_preferences(app);
            }
            "timer_cancel" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("start-timer", 0);
                }
            }
            "snooze" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("snooze-alerts", 60);
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                show_preferences(app);
            }
        })
        .build(app)?;

    Ok(())
}

pub fn show_preferences(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
