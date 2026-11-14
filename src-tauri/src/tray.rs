use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub struct TrayMenuItems {
    pub status: MenuItem<tauri::Wry>,
    pub next_event: MenuItem<tauri::Wry>,
    pub snooze: MenuItem<tauri::Wry>,
    pub resume: MenuItem<tauri::Wry>,
}

const SNOOZE_LABEL: &str = "Snooze Alerts (1 hour)";

pub static TRAY_ITEMS: Mutex<Option<TrayMenuItems>> = Mutex::new(None);

pub fn update_tray_status(text: &str) {
    if let Ok(guard) = TRAY_ITEMS.lock()
        && let Some(items) = guard.as_ref()
    {
        let _ = items.status.set_text(text);
    }
}

/// Shows the snooze state: "Alerts snoozed until 2:05 PM" (or 14:05, as the
/// system clock is set) plus an enabled "Resume Alerts", or the normal
/// snooze item.
pub fn show_snooze(until: Option<i64>) {
    if let Ok(guard) = TRAY_ITEMS.lock()
        && let Some(items) = guard.as_ref()
    {
        match until.and_then(|t| chrono::DateTime::from_timestamp(t, 0)) {
            Some(t) => {
                let local = t.with_timezone(&chrono::Local);
                let _ = items.snooze.set_text(format!(
                    "Alerts snoozed until {}",
                    crate::snooze::format_clock(local)
                ));
                let _ = items.snooze.set_enabled(false);
                let _ = items.resume.set_enabled(true);
            }
            None => {
                let _ = items.snooze.set_text(SNOOZE_LABEL);
                let _ = items.snooze.set_enabled(true);
                let _ = items.resume.set_enabled(false);
            }
        }
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

    let snooze_item = MenuItem::with_id(app, "snooze", SNOOZE_LABEL, true, None::<&str>)?;
    let resume_item =
        MenuItem::with_id(app, "resume_alerts", "Resume Alerts", false, None::<&str>)?;

    if let Ok(mut guard) = TRAY_ITEMS.lock() {
        *guard = Some(TrayMenuItems {
            status: status_item.clone(),
            next_event: next_event_item.clone(),
            snooze: snooze_item.clone(),
            resume: resume_item.clone(),
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
            &resume_item,
            &sync_item,
            &sep3,
            &open_web_item,
            &prefs_item,
            &updates_item,
            &sep4,
            &quit_item,
        ],
    )?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("CubiPal Desktop Companion");

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    let _tray = builder
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
                // The window asks for the number of minutes.
                show_preferences(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("custom-focus", ());
                }
            }
            "timer_cancel" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.emit("start-timer", 0);
                }
            }
            "snooze" => crate::snooze::snooze(app, 60),
            "resume_alerts" => crate::snooze::resume(app),
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

/// Whether the desktop can show our tray icon.
///
/// On Linux the tray icon is a StatusNotifierItem, which only appears if a
/// host has registered `org.kde.StatusNotifierWatcher` on the session bus.
/// ChromeOS (Crostini) and stock GNOME have no such host, so the icon never
/// shows and the app would be unreachable. Desktops with only a legacy
/// XEmbed tray report `false` too; they just get the window at startup.
///
/// Windows and macOS always have a tray / menu bar.
#[cfg(target_os = "linux")]
pub fn tray_host_available() -> bool {
    use dbus::blocking::Connection;
    use std::time::Duration;

    let Ok(conn) = Connection::new_session() else {
        return false;
    };
    let proxy = conn.with_proxy(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        Duration::from_millis(500),
    );
    let reply: Result<(bool,), dbus::Error> = proxy.method_call(
        "org.freedesktop.DBus",
        "NameHasOwner",
        ("org.kde.StatusNotifierWatcher",),
    );
    matches!(reply, Ok((true,)))
}

#[cfg(not(target_os = "linux"))]
pub fn tray_host_available() -> bool {
    true
}

pub fn show_preferences(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
