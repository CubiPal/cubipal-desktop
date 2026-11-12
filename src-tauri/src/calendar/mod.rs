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
    /// False when this build has no Microsoft client ID (Outlook support
    /// is "coming soon" in the UI).
    pub microsoft_available: bool,
    pub ics_url: Option<String>,
}

/// Why fetching one calendar failed.
#[derive(Debug)]
pub enum ProviderError {
    /// The refresh token was revoked or expired (`invalid_grant`): the
    /// account must be connected again.
    Revoked,
    Failed(String),
}

impl From<String> for ProviderError {
    fn from(s: String) -> Self {
        ProviderError::Failed(s)
    }
}

/// True for an OAuth token-endpoint error meaning the refresh token is dead.
pub fn is_revoked_grant(status: u16, body: &str) -> bool {
    status == 400 && body.contains("invalid_grant")
}

pub const MICROSOFT_COMING_SOON: &str = "Outlook / Microsoft 365 support is coming soon.";

#[tauri::command]
pub async fn connect_google_account(custom_client_id: Option<String>) -> Result<(), String> {
    let client = match custom_client_id.filter(|s| !s.trim().is_empty()) {
        Some(id) => google::OAuthClient::custom(id),
        None => google::OAuthClient::built_in(),
    };

    let (_access, refresh) = google::start_google_auth(&client).await?;

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
        .map(|s| s.trim().to_string())
        .filter(|s| microsoft::is_usable_client_id(s))
        .or_else(|| microsoft::built_in_client_id().map(String::from))
        .ok_or_else(|| MICROSOFT_COMING_SOON.to_string())?;

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
        microsoft_available: microsoft::built_in_client_id().is_some(),
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
        let result = google::fetch_google_events(&google::OAuthClient::built_in(), &refresh).await;
        collect(
            result,
            "Google Calendar",
            KEYRING_GOOGLE_REFRESH,
            &mut all_events,
            &mut errors,
        );
    }

    // 2. Microsoft 365 (only in builds that have a Microsoft client ID)
    if let Some(refresh) = read_secret(KEYRING_MICROSOFT_REFRESH)
        && let Some(client_id) = microsoft::built_in_client_id()
    {
        let result = microsoft::fetch_microsoft_events(client_id, &refresh).await;
        collect(
            result,
            "Microsoft 365",
            KEYRING_MICROSOFT_REFRESH,
            &mut all_events,
            &mut errors,
        );
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

/// Adds one provider's events, or its error. A revoked sign-in also forgets
/// the dead refresh token, so the Calendars tab shows "Not connected".
fn collect(
    result: Result<Vec<CalendarEvent>, ProviderError>,
    name: &str,
    keyring_user: &str,
    all_events: &mut Vec<CalendarEvent>,
    errors: &mut Vec<String>,
) {
    match result {
        Ok(events) => all_events.extend(events),
        Err(ProviderError::Revoked) => {
            if let Ok(entry) = Entry::new(KEYRING_SERVICE, keyring_user) {
                let _ = entry.delete_credential();
            }
            errors.push(format!(
                "{}: sign-in expired or access was removed. Connect it again in Calendars.",
                name
            ));
        }
        Err(ProviderError::Failed(e)) => errors.push(format!("{}: {}", name, e)),
    }
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

impl DevicePushEvent {
    /// Builds the event sent to the Pal. Text is reduced to what the
    /// firmware's simple JSON scanner and ASCII display can handle.
    pub fn from_event(e: &CalendarEvent, chime_lead_min: u8) -> Self {
        Self {
            id: Some(sanitize_device_id(&e.id))
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| format!("evt_{}", e.start)),
            title: sanitize_device_title(&e.title),
            start: e.start,
            end: e.end,
            chime_lead_min,
        }
    }
}

/// Longest title sent to the Pal (its display shows about 20 characters).
const DEVICE_TITLE_MAX_CHARS: usize = 60;
const DEVICE_ID_MAX_CHARS: usize = 64;

/// The firmware parses `/api/events` with a naive scanner: an object ends at
/// the first `}`, a string at the first `"`, and `\uXXXX` escapes are shown
/// literally; its display only has printable ASCII. So keep printable ASCII
/// minus `"` `\` `{` `}` `[` `]`, fold common accents/punctuation, turn
/// whitespace runs into one space, and cap the length.
pub fn sanitize_device_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars() {
        let mapped: &str = match c {
            '"' | '\\' | '{' | '}' | '[' | ']' => "",
            c if c.is_whitespace() || c.is_control() => " ",
            '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}' | '`' => "'",
            '\u{2013}' | '\u{2014}' => "-",
            '\u{2026}' => "...",
            c if c.is_ascii() => {
                out.push(c);
                continue;
            }
            c => fold_latin(c),
        };
        out.push_str(mapped);
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(DEVICE_TITLE_MAX_CHARS).collect();
    let trimmed = truncated.trim();
    if trimmed.is_empty() {
        "Meeting".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Event ids only need to be stable; keep a safe ASCII subset.
pub fn sanitize_device_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':'))
        .take(DEVICE_ID_MAX_CHARS)
        .collect()
}

/// Drops accents from common Latin letters; anything else is removed.
fn fold_latin(c: char) -> &'static str {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "A",
        'æ' => "ae",
        'Æ' => "AE",
        'ç' => "c",
        'Ç' => "C",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'È' | 'É' | 'Ê' | 'Ë' => "E",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'ñ' => "n",
        'Ñ' => "N",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => "o",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => "O",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'Ù' | 'Ú' | 'Û' | 'Ü' => "U",
        'ý' | 'ÿ' => "y",
        'Ý' => "Y",
        'ß' => "ss",
        _ => "",
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DevicePushPayload {
    events: Vec<DevicePushEvent>,
}

#[tauri::command]
pub async fn test_device_chime() -> Result<(), String> {
    let target = crate::device::paired_target().await?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .post(format!("http://{}/api/message", target.ip))
        .bearer_auth(&target.token)
        .header("X-Pet", "1")
        .form(&[("text", "Meeting Chime Test")])
        .send()
        .await
        .map_err(crate::device::reach_error)?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(crate::device::device_error(res).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoked_grant_is_recognised() {
        assert!(is_revoked_grant(
            400,
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#
        ));
        assert!(!is_revoked_grant(400, r#"{"error":"invalid_request"}"#));
        assert!(!is_revoked_grant(500, "invalid_grant"));
    }

    #[test]
    fn titles_are_safe_for_the_firmware_parser() {
        assert_eq!(sanitize_device_title("Sync {team}"), "Sync team");
        assert_eq!(
            sanitize_device_title(r#"Say "hi" \ [draft]"#),
            "Say hi draft"
        );
        assert_eq!(
            sanitize_device_title("Line1\nLine2\t end "),
            "Line1 Line2 end"
        );
        assert_eq!(
            sanitize_device_title("Café Réunion – Q4…"),
            "Cafe Reunion - Q4..."
        );
        assert_eq!(sanitize_device_title("Bob’s 1:1 🎉"), "Bob's 1:1");
        assert_eq!(sanitize_device_title("🎉 {}"), "Meeting");
        assert_eq!(sanitize_device_title(""), "Meeting");
        let long = "x".repeat(200);
        assert_eq!(sanitize_device_title(&long).len(), DEVICE_TITLE_MAX_CHARS);
        let out = sanitize_device_title("Ünïcödé \"quoted\" {x} ✓ end");
        assert!(out.chars().all(|c| (' '..='~').contains(&c)));
        assert!(!out.contains(['"', '\\', '{', '}', '[', ']']));
    }

    #[test]
    fn ids_are_safe_and_bounded() {
        assert_eq!(
            sanitize_device_id("abc_123-x@google.com"),
            "abc_123-x@google.com"
        );
        assert_eq!(sanitize_device_id("{\"evil\"}\\é"), "evil");
        assert_eq!(
            sanitize_device_id(&"a".repeat(100)).len(),
            DEVICE_ID_MAX_CHARS
        );
    }

    #[test]
    fn push_payload_round_trips_through_naive_scanner_shape() {
        let e = CalendarEvent {
            id: "id\"1".into(),
            title: "Plan {Q4} \"launch\"".into(),
            start: 100,
            end: 200,
            source: "google".into(),
            is_all_day: false,
            is_tentative: false,
        };
        let json = serde_json::to_string(&DevicePushEvent::from_event(&e, 5)).unwrap();
        // One object, no inner braces or escaped quotes for the firmware to trip on.
        assert_eq!(json.matches('{').count(), 1);
        assert_eq!(json.matches('}').count(), 1);
        assert!(!json.contains('\\'));
        assert!(json.contains(r#""title":"Plan Q4 launch""#));
    }

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
