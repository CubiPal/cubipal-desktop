pub mod google;
pub mod ics;
pub mod microsoft;
pub mod oauth;
pub mod sync_engine;

pub use sync_engine::{
    CalendarSyncConfig, get_saved_calendar_config, save_calendar_config_to_keyring,
    start_background_sync, trigger_immediate_sync,
};

use chrono::Utc;
use keyring::Entry;
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "cubipal";
const KEYRING_GOOGLE_REFRESH: &str = "oauth_google_refresh";
const KEYRING_MICROSOFT_REFRESH: &str = "oauth_microsoft_refresh";
const KEYRING_ICS_URL: &str = "calendar_ics_url";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: i64, // Unix epoch timestamp
    pub end: i64,
    pub source: String,
    pub is_all_day: bool,
    pub is_tentative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSyncSettings {
    pub private_mode: bool,
    pub skip_all_day: bool,
    pub skip_tentative: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedAccounts {
    pub google_connected: bool,
    pub microsoft_connected: bool,
    pub ics_url: Option<String>,
}

#[tauri::command]
pub async fn connect_google_account(custom_client_id: Option<String>) -> Result<(), String> {
    let client_id = custom_client_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| google::GOOGLE_DEFAULT_CLIENT_ID.to_string());

    let (_access, refresh) = google::start_google_auth(&client_id).await?;

    let entry = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH).map_err(|e| e.to_string())?;
    entry.set_password(&refresh).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn disconnect_google_account() -> Result<(), String> {
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

#[tauri::command]
pub async fn connect_microsoft_account(custom_client_id: Option<String>) -> Result<(), String> {
    let client_id = custom_client_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| microsoft::MICROSOFT_DEFAULT_CLIENT_ID.to_string());

    let (_access, refresh) = microsoft::start_microsoft_auth(&client_id).await?;

    let entry =
        Entry::new(KEYRING_SERVICE, KEYRING_MICROSOFT_REFRESH).map_err(|e| e.to_string())?;
    entry.set_password(&refresh).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn disconnect_microsoft_account() -> Result<(), String> {
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_MICROSOFT_REFRESH) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

#[tauri::command]
pub async fn set_ics_url(url: String) -> Result<(), String> {
    let clean = url.trim();
    if clean.is_empty() {
        return disconnect_ics().await;
    }

    let normalized = ics::normalize_feed_url(clean)?;
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_ICS_URL).map_err(|e| e.to_string())?;
    entry.set_password(&normalized).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn disconnect_ics() -> Result<(), String> {
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_ICS_URL) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

#[tauri::command]
pub fn get_connected_accounts() -> Result<ConnectedAccounts, String> {
    let google_connected = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH)
        .and_then(|e| e.get_password())
        .is_ok();

    let microsoft_connected = Entry::new(KEYRING_SERVICE, KEYRING_MICROSOFT_REFRESH)
        .and_then(|e| e.get_password())
        .is_ok();

    let ics_url = Entry::new(KEYRING_SERVICE, KEYRING_ICS_URL)
        .and_then(|e| e.get_password())
        .ok();

    Ok(ConnectedAccounts {
        google_connected,
        microsoft_connected,
        ics_url,
    })
}

#[tauri::command]
pub fn get_calendar_sync_config() -> Result<CalendarSyncConfig, String> {
    Ok(get_saved_calendar_config())
}

#[tauri::command]
pub fn save_calendar_sync_config(config: CalendarSyncConfig) -> Result<(), String> {
    save_calendar_config_to_keyring(&config)?;
    trigger_immediate_sync();
    Ok(())
}

#[tauri::command]
pub fn trigger_calendar_sync() -> Result<(), String> {
    trigger_immediate_sync();
    Ok(())
}

fn read_secret(user: &str) -> Option<String> {
    Entry::new(KEYRING_SERVICE, user)
        .and_then(|e| e.get_password())
        .ok()
}

/// Fetches every configured calendar. If any configured source fails the
/// whole sync fails, so callers never mistake an error (expired token,
/// network blip) for "no meetings" and wipe the events already on the Pal.
#[tauri::command]
pub async fn sync_calendar_events(
    settings: CalendarSyncSettings,
) -> Result<Vec<CalendarEvent>, String> {
    let mut all_events = Vec::new();
    let mut errors = Vec::new();

    // 1. Google Calendar
    if let Some(refresh) = read_secret(KEYRING_GOOGLE_REFRESH) {
        match google::fetch_google_events(google::GOOGLE_DEFAULT_CLIENT_ID, &refresh).await {
            Ok(events) => all_events.extend(events),
            Err(e) => errors.push(format!("Google Calendar: {}", e)),
        }
    }

    // 2. Microsoft 365
    if let Some(refresh) = read_secret(KEYRING_MICROSOFT_REFRESH) {
        match microsoft::fetch_microsoft_events(microsoft::MICROSOFT_DEFAULT_CLIENT_ID, &refresh)
            .await
        {
            Ok(events) => all_events.extend(events),
            Err(e) => errors.push(format!("Microsoft 365: {}", e)),
        }
    }

    // 3. ICS Calendar Feed
    if let Some(url) = read_secret(KEYRING_ICS_URL) {
        match ics::fetch_ics_events(&url).await {
            Ok(events) => all_events.extend(events),
            Err(e) => errors.push(format!("Calendar link: {}", e)),
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("; "));
    }

    Ok(filter_events(all_events, &settings, Utc::now().timestamp()))
}

/// Filter pipeline: drop events that have ended, optionally all-day and
/// tentative ones; sort by start; apply Private Mode.
fn filter_events(
    events: Vec<CalendarEvent>,
    settings: &CalendarSyncSettings,
    now: i64,
) -> Vec<CalendarEvent> {
    let mut filtered: Vec<CalendarEvent> = events
        .into_iter()
        .filter(|e| e.end > now)
        .filter(|e| !settings.skip_all_day || !e.is_all_day)
        .filter(|e| !settings.skip_tentative || !e.is_tentative)
        .collect();

    // Sort by start time ascending
    filtered.sort_by_key(|e| e.start);

    // Apply Private Mode transformation
    if settings.private_mode {
        for event in &mut filtered {
            event.title = "Busy".to_string();
        }
    }

    filtered
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePushEvent {
    pub id: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    #[serde(rename = "chimeLeadMin")]
    pub chime_lead_min: u8,
}

#[derive(Debug, Serialize, Deserialize)]
struct DevicePushPayload {
    events: Vec<DevicePushEvent>,
}

#[tauri::command]
pub async fn push_events_to_device(
    events: Vec<CalendarEvent>,
    lead_time_min: u8,
) -> Result<bool, String> {
    let (ip, token) = match crate::device::get_saved_device_credentials() {
        Some(creds) => creds,
        None => return Ok(false),
    };

    let push_events: Vec<DevicePushEvent> = events
        .into_iter()
        .take(2)
        .map(|e| DevicePushEvent {
            id: e.id,
            title: e.title,
            start: e.start,
            end: e.end,
            chime_lead_min: lead_time_min,
        })
        .collect();

    let payload = DevicePushPayload {
        events: push_events,
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .put(format!("http://{}/api/events", ip))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Pet", "1")
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to reach CubiPal: {}", e))?;

    if res.status().is_success() {
        Ok(true)
    } else {
        Err(format!("Device returned HTTP {}", res.status()))
    }
}

#[tauri::command]
pub async fn test_device_chime() -> Result<(), String> {
    let (ip, token) = crate::device::get_saved_device_credentials()
        .ok_or_else(|| "No CubiPal is currently paired.".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .post(format!("http://{}/api/message", ip))
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Pet", "1")
        .form(&[("text", "Meeting Chime Test")])
        .send()
        .await
        .map_err(|e| format!("Failed to reach CubiPal: {}", e))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(format!("Device returned HTTP {}", res.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, start: i64, end: i64, all_day: bool, tentative: bool) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            title: format!("Meeting {id}"),
            start,
            end,
            source: "ics".into(),
            is_all_day: all_day,
            is_tentative: tentative,
        }
    }

    #[test]
    fn filters_sorts_and_privatises() {
        let events = vec![
            ev("later", 300, 400, false, false),
            ev("ended", 0, 100, false, false),
            ev("allday", 150, 500, true, false),
            ev("tent", 200, 300, false, true),
            ev("now", 50, 200, false, false),
        ];
        let settings = CalendarSyncSettings {
            private_mode: false,
            skip_all_day: true,
            skip_tentative: true,
        };
        let ids: Vec<_> = filter_events(events.clone(), &settings, 100)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["now", "later"]);

        let settings = CalendarSyncSettings {
            private_mode: true,
            skip_all_day: false,
            skip_tentative: false,
        };
        let out = filter_events(events, &settings, 100);
        let ids: Vec<_> = out.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["now", "allday", "tent", "later"]);
        assert!(out.iter().all(|e| e.title == "Busy"));
    }
}
