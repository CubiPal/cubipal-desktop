//! App preferences shown on the General tab.
//!
//! "Launch at Startup" lives in the OS (the login item / `Run` registry
//! value that tauri-plugin-autostart manages), so it is read back from there
//! rather than stored twice. Other preferences are kept in `settings.json`
//! in the app's config directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Check for new app versions at startup and every 24 h, and offer them.
    pub auto_update: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings { auto_update: true }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneralSettings {
    pub autostart: bool,
    pub auto_update: bool,
    pub app_version: String,
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("settings.json"))
}

/// Parses saved settings; anything unreadable gives the defaults.
fn parse_settings(text: &str) -> AppSettings {
    serde_json::from_str(text).unwrap_or_default()
}

pub fn load(app: &AppHandle) -> AppSettings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_settings(&t))
        .unwrap_or_default()
}

fn save(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app).ok_or("No settings folder is available")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't save settings: {}", e))?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    // Write then rename, so a crash can't leave a half-written file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("Couldn't save settings: {}", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("Couldn't save settings: {}", e))
}

#[tauri::command]
pub fn get_general_settings(app: AppHandle) -> GeneralSettings {
    GeneralSettings {
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        auto_update: load(&app).auto_update,
        app_version: app.package_info().version.to_string(),
    }
}

/// Turns launch-at-login on or off; returns the state the OS now reports.
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| format!("Couldn't change Launch at Startup: {}", e))?;
    launcher.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_auto_update(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = load(&app);
    settings.auto_update = enabled;
    save(&app, &settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_round_trip() {
        assert!(parse_settings("").auto_update);
        assert!(parse_settings("not json").auto_update);
        assert!(parse_settings("{}").auto_update);
        assert!(!parse_settings(r#"{"auto_update":false}"#).auto_update);
        // Unknown keys from a newer version are ignored.
        assert!(parse_settings(r#"{"auto_update":true,"future":1}"#).auto_update);
        let s = AppSettings { auto_update: false };
        assert_eq!(parse_settings(&serde_json::to_string(&s).unwrap()), s);
    }
}
