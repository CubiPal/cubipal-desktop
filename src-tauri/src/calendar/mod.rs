pub mod google;
pub mod ics;
pub mod microsoft;
pub mod oauth;

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

    let entry = Entry::new(KEYRING_SERVICE, KEYRING_ICS_URL).map_err(|e| e.to_string())?;
    entry.set_password(clean).map_err(|e| e.to_string())?;
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
pub async fn sync_calendar_events(
    settings: CalendarSyncSettings,
) -> Result<Vec<CalendarEvent>, String> {
    let mut all_events = Vec::new();
    let now = Utc::now().timestamp();

    // 1. Google Calendar
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_GOOGLE_REFRESH) {
        if let Ok(refresh) = entry.get_password() {
            if let Ok(events) =
                google::fetch_google_events(google::GOOGLE_DEFAULT_CLIENT_ID, &refresh).await
            {
                all_events.extend(events);
            }
        }
    }

    // 2. Microsoft 365
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_MICROSOFT_REFRESH) {
        if let Ok(refresh) = entry.get_password() {
            if let Ok(events) =
                microsoft::fetch_microsoft_events(microsoft::MICROSOFT_DEFAULT_CLIENT_ID, &refresh)
                    .await
            {
                all_events.extend(events);
            }
        }
    }

    // 3. ICS Calendar Feed
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_ICS_URL) {
        if let Ok(url) = entry.get_password() {
            if let Ok(events) = ics::fetch_ics_events(&url).await {
                all_events.extend(events);
            }
        }
    }

    // Filter pipeline:
    // - Must not have ended in the past
    // - Skip all-day if toggle enabled
    // - Skip tentative if toggle enabled
    let mut filtered: Vec<CalendarEvent> = all_events
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

    Ok(filtered)
}
