use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

pub fn create_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let status_item = MenuItem::with_id(app, "status", "⚪ CubiPal — Offline", false, None::<&str>)?;
    let next_event_item = MenuItem::with_id(app, "next_event", "Next: No upcoming events", false, None::<&str>)?;

    // Focus Timer Submenu (presets: 30m, 60m, custom, cancel)
    let timer_30 = MenuItem::with_id(app, "timer_30", "30 min Focus", true, None::<&str>)?;
    let timer_60 = MenuItem::with_id(app, "timer_60", "60 min Deep Work", true, None::<&str>)?;
    let timer_custom = MenuItem::with_id(app, "timer_custom", "Custom Focus...", true, None::<&str>)?;
    let timer_cancel = MenuItem::with_id(app, "timer_cancel", "Cancel Active Timer", true, None::<&str>)?;

    let timer_sub = Submenu::with_items(
        app,
        "Focus Timer",
        true,
        &[&timer_30, &timer_60, &timer_custom, &timer_cancel],
    )?;

    let snooze_item = MenuItem::with_id(app, "snooze", "Snooze Alerts (1 hour)", true, None::<&str>)?;
    let sync_item = MenuItem::with_id(app, "sync_now", "Sync Calendars Now", true, None::<&str>)?;

    let open_web_item = MenuItem::with_id(app, "open_web", "Open Device Web Control", true, None::<&str>)?;
    let prefs_item = MenuItem::with_id(app, "preferences", "Preferences...", true, None::<&str>)?;
    let updates_item = MenuItem::with_id(app, "check_updates", "Check for Updates...", true, None::<&str>)?;
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
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "preferences" => {
                    show_preferences(app);
                }
                "open_web" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.emit("open-device-web", ());
                    }
                }
                "check_updates" => {
                    let _ = open::that("https://github.com/houndman1017/cubipal-desktop/releases");
                }
                "sync_now" => {
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
            }
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
