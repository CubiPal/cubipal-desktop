use chrono::Utc;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, mpsc};

use super::{
    CalendarEvent, CalendarSyncSettings, DevicePushEvent, DevicePushPayload, KEYRING_SERVICE,
};

pub const KEYRING_CALENDAR_CONFIG: &str = "calendar_sync_config";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarSyncConfig {
    pub private_mode: bool,
    pub skip_all_day: bool,
    pub skip_tentative: bool,
    #[serde(rename = "leadTimeMin")]
    pub lead_time_min: u8,
}

impl Default for CalendarSyncConfig {
    fn default() -> Self {
        Self {
            private_mode: false,
            skip_all_day: true,
            skip_tentative: true,
            lead_time_min: 5,
        }
    }
}

pub struct SyncEngine {
    trigger_tx: mpsc::Sender<()>,
    #[allow(dead_code)]
    pending_queue: Arc<Mutex<Option<Vec<DevicePushEvent>>>>,
}

static SYNC_ENGINE: Mutex<Option<SyncEngine>> = Mutex::const_new(None);

pub fn get_saved_calendar_config() -> CalendarSyncConfig {
    if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_CALENDAR_CONFIG)
        && let Ok(json_str) = entry.get_password()
        && let Ok(cfg) = serde_json::from_str::<CalendarSyncConfig>(&json_str)
    {
        return cfg;
    }
    CalendarSyncConfig::default()
}

pub fn save_calendar_config_to_keyring(cfg: &CalendarSyncConfig) -> Result<(), String> {
    let entry = Entry::new(KEYRING_SERVICE, KEYRING_CALENDAR_CONFIG).map_err(|e| e.to_string())?;
    let json_str = serde_json::to_string(cfg).map_err(|e| e.to_string())?;
    entry.set_password(&json_str).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn trigger_immediate_sync() {
    tauri::async_runtime::spawn(async {
        if let Some(engine) = SYNC_ENGINE.lock().await.as_ref() {
            let _ = engine.trigger_tx.send(()).await;
        }
    });
}

pub fn start_background_sync(app: AppHandle) {
    let (tx, mut rx) = mpsc::channel::<()>(8);
    let pending_queue = Arc::new(Mutex::new(None));

    let engine = SyncEngine {
        trigger_tx: tx,
        pending_queue: pending_queue.clone(),
    };

    tauri::async_runtime::spawn(async move {
        *SYNC_ENGINE.lock().await = Some(engine);
    });

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval_5min = tokio::time::interval(Duration::from_secs(300));
        let mut interval_retry = tokio::time::interval(Duration::from_secs(30));

        // Initial sync on startup after 3 seconds
        tokio::time::sleep(Duration::from_secs(3)).await;
        perform_sync_cycle(&app_clone, &pending_queue).await;

        loop {
            tokio::select! {
                _ = rx.recv() => {
                    // Manual or event-triggered sync
                    perform_sync_cycle(&app_clone, &pending_queue).await;
                }
                _ = interval_5min.tick() => {
                    // Periodic 5-minute background sync
                    perform_sync_cycle(&app_clone, &pending_queue).await;
                }
                _ = interval_retry.tick() => {
                    // Fast 30s retry if device was offline and has queued events
                    retry_pending_push(&app_clone, &pending_queue).await;
                }
            }
        }
    });
}

async fn perform_sync_cycle(
    app: &AppHandle,
    pending_queue: &Arc<Mutex<Option<Vec<DevicePushEvent>>>>,
) {
    let config = get_saved_calendar_config();
    let sync_settings = CalendarSyncSettings {
        private_mode: config.private_mode,
        skip_all_day: config.skip_all_day,
        skip_tentative: config.skip_tentative,
    };

    // 1. Fetch and filter events
    let events = match super::sync_calendar_events(sync_settings).await {
        Ok(evts) => evts,
        Err(e) => {
            // Keep the last good event set on the Pal; an error is not "no meetings".
            eprintln!("[SyncEngine] Failed to sync calendars, not pushing: {}", e);
            return;
        }
    };

    let _ = app.emit("calendar-events-updated", &events);

    // 2. Format next 2 events for CubiPal
    let push_events: Vec<DevicePushEvent> = events
        .iter()
        .take(2)
        .map(|e| DevicePushEvent::from_event(e, config.lead_time_min))
        .collect();

    // 3. Update Tray Status & Upcoming Event (full titles; the tray can show Unicode)
    update_tray_display(&events);

    // 4. Push to Paired Device
    let push_result = push_events_payload(&push_events).await;
    match push_result {
        Ok(true) => {
            // Success
            *pending_queue.lock().await = None;
            crate::tray::update_tray_status("🟢 CubiPal — Online");
        }
        Ok(false) => {
            // Not paired
            *pending_queue.lock().await = None;
            crate::tray::update_tray_status("⚪ CubiPal — Not Paired");
        }
        Err(err) => {
            // Offline / timeout: queue for fast retry
            eprintln!("[SyncEngine] Device push offline: {}. Queuing retry.", err);
            *pending_queue.lock().await = Some(push_events);
            crate::tray::update_tray_status("⚪ CubiPal — Offline");
        }
    }
}

async fn retry_pending_push(
    _app: &AppHandle,
    pending_queue: &Arc<Mutex<Option<Vec<DevicePushEvent>>>>,
) {
    let mut lock = pending_queue.lock().await;
    if let Some(ref events) = *lock {
        if events.is_empty() {
            *lock = None;
            return;
        }

        let push_result = push_events_payload(events).await;
        if let Ok(true) = push_result {
            println!("[SyncEngine] Offline retry succeeded! CubiPal received queued events.");
            *lock = None;
            crate::tray::update_tray_status("🟢 CubiPal — Online");
        }
    }
}

async fn push_events_payload(events: &[DevicePushEvent]) -> Result<bool, String> {
    let (ip, token) = match crate::device::get_saved_device_credentials() {
        Some(creds) => creds,
        None => return Ok(false),
    };

    let payload = DevicePushPayload {
        events: events.to_vec(),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
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
        .map_err(|e| format!("Network error: {}", e))?;

    if res.status().is_success() {
        Ok(true)
    } else {
        Err(format!("Device returned HTTP {}", res.status()))
    }
}

fn update_tray_display(events: &[CalendarEvent]) {
    let text = match events.first() {
        Some(next) => tray_text(&next.title, next.start, next.end, Utc::now().timestamp()),
        None => "Next: No upcoming events".to_string(),
    };
    crate::tray::update_tray_next_event(&text);
}

/// The tray's "next event" line for an event at `start..end` seen at `now`.
fn tray_text(title: &str, start: i64, end: i64, now: i64) -> String {
    let rem_min = (start - now) / 60;
    if now >= start && now < end {
        let mins_left = (end - now) / 60;
        format!("Now: {} ({}m left)", truncate_str(title, 18), mins_left)
    } else if rem_min <= 0 {
        format!("Next: {} (starting now)", truncate_str(title, 18))
    } else if rem_min < 60 {
        format!("Next: {} (in {}m)", truncate_str(title, 18), rem_min)
    } else {
        format!(
            "Next: {} (in {}h {}m)",
            truncate_str(title, 16),
            rem_min / 60,
            rem_min % 60
        )
    }
}

/// Shortens `s` to at most `max_len` characters (not bytes), ending in `…`.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        return s.to_string();
    }
    if max_len == 0 {
        return String::new();
    }
    let truncated: String = s.chars().take(max_len - 1).collect();
    format!("{}…", truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_counts_chars_not_bytes() {
        assert_eq!(truncate_str("short", 18), "short");
        assert_eq!(truncate_str("exactly5", 8), "exactly5");
        assert_eq!(truncate_str("123456789", 5), "1234…");
        assert_eq!(truncate_str("üüüüüü", 4), "üüü…");
        assert_eq!(truncate_str("🎉🎉🎉", 2), "🎉…");
        assert_eq!(truncate_str("abc", 1), "…");
        assert_eq!(truncate_str("abc", 0), "");
        assert_eq!(truncate_str("", 0), "");
    }

    #[test]
    fn tray_text_states() {
        let now = 1_000_000;
        assert_eq!(
            tray_text("Standup", now - 60, now + 600, now),
            "Now: Standup (10m left)"
        );
        assert_eq!(
            tray_text("Standup", now + 30, now + 600, now),
            "Next: Standup (starting now)"
        );
        assert_eq!(
            tray_text("Standup", now + 59 * 60, now + 3600, now),
            "Next: Standup (in 59m)"
        );
        assert_eq!(
            tray_text("Standup", now + 3600, now + 7200, now),
            "Next: Standup (in 1h 0m)"
        );
        assert_eq!(
            tray_text(
                "A very long meeting title indeed",
                now + 7500,
                now + 9000,
                now
            ),
            "Next: A very long mee… (in 2h 5m)"
        );
    }

    #[test]
    fn sync_config_serde_round_trip() {
        let cfg = CalendarSyncConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"leadTimeMin\":5"));
        let back: CalendarSyncConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.lead_time_min, 5);
        assert!(back.skip_all_day && back.skip_tentative && !back.private_mode);
    }
}
