//! Tray "Snooze Alerts (1 hour)" (ENG-29, TST-15).
//!
//! While snoozed:
//! - The Pal gets no meeting that starts before the snooze ends. The
//!   firmware treats `chimeLeadMin: 0` as "no reminder before the start" but
//!   still chimes when the meeting starts, so 0 alone wouldn't silence it;
//!   leaving those meetings out of the push is the closest safe option.
//! - Later meetings are pushed with their reminder shortened so it can't
//!   ring before the snooze ends.
//! - The app's own unprompted pop-ups (the background update offer) wait.
//!
//! Snooze lasts until it expires or "Resume Alerts" is chosen (tray or the
//! Device tab); it is not kept across restarts. Meetings that started while
//! snoozed stay off the Pal after it ends, so it doesn't ring late (B-8).

use crate::calendar::{CalendarEvent, DevicePushEvent};
use chrono::Utc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static SNOOZE_UNTIL: Mutex<Option<i64>> = Mutex::new(None);
/// When the last snooze ended (expired or resumed).
static SNOOZE_ENDED_AT: Mutex<Option<i64>> = Mutex::new(None);
/// Bumped on every change, so an old expiry timer can't end a newer snooze.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The user's clock uses 12-hour time (reported by the window from the
/// system locale; 24-hour until then).
static TWELVE_HOUR: AtomicBool = AtomicBool::new(false);

/// End of the current snooze (epoch seconds), if one is active.
pub fn snoozed_until() -> Option<i64> {
    let now = Utc::now().timestamp();
    SNOOZE_UNTIL
        .lock()
        .ok()
        .and_then(|g| *g)
        .filter(|&until| until > now)
}

pub fn is_snoozed() -> bool {
    snoozed_until().is_some()
}

/// When the last snooze ended, if one did.
pub fn snooze_ended_at() -> Option<i64> {
    SNOOZE_ENDED_AT.lock().ok().and_then(|g| *g)
}

/// A local time of day in the user's 12- or 24-hour format ("2:05 PM").
pub fn format_clock(t: chrono::DateTime<chrono::Local>) -> String {
    if TWELVE_HOUR.load(Ordering::Relaxed) {
        t.format("%-I:%M %p").to_string()
    } else {
        t.format("%H:%M").to_string()
    }
}

fn notify(app: &AppHandle, until: Option<i64>) {
    crate::tray::show_snooze(until);
    let _ = app.emit("snooze-changed", until);
}

/// The end of the current snooze (epoch seconds), for the Device tab.
#[tauri::command]
pub fn get_snooze_state() -> Option<i64> {
    snoozed_until()
}

/// "Resume" in the Device tab's snooze banner.
#[tauri::command]
pub fn resume_alerts(app: AppHandle) {
    resume(&app);
}

/// The window reports whether the system locale uses a 12-hour clock.
#[tauri::command]
pub fn set_clock_format(twelve_hour: bool) {
    TWELVE_HOUR.store(twelve_hour, Ordering::Relaxed);
    if let Some(until) = snoozed_until() {
        crate::tray::show_snooze(Some(until));
    }
}

pub fn snooze(app: &AppHandle, minutes: i64) {
    let until = Utc::now().timestamp() + minutes * 60;
    if let Ok(mut g) = SNOOZE_UNTIL.lock() {
        *g = Some(until);
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    notify(app, Some(until));
    crate::calendar::trigger_immediate_sync();

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs((minutes * 60).max(0) as u64)).await;
        if GENERATION.load(Ordering::SeqCst) == generation {
            resume(&app);
        }
    });
}

pub fn resume(app: &AppHandle) {
    let was_snoozed = SNOOZE_UNTIL
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .is_some();
    if was_snoozed && let Ok(mut g) = SNOOZE_ENDED_AT.lock() {
        *g = Some(Utc::now().timestamp());
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
    notify(app, None);
    crate::calendar::trigger_immediate_sync();
}

/// The (at most `max`) events to push to the Pal, honouring a snooze that
/// ends at `snooze_until`. After a snooze that ended at `snooze_ended_at`,
/// meetings that had already started by then aren't pushed: they were held
/// back, and the Pal would ring a late "started" alert for them (B-8).
pub fn push_events(
    events: &[CalendarEvent],
    lead_min: u8,
    snooze_until: Option<i64>,
    snooze_ended_at: Option<i64>,
    max: usize,
) -> Vec<DevicePushEvent> {
    events
        .iter()
        .filter_map(|e| match snooze_until {
            None if snooze_ended_at.is_some_and(|ended| e.start < ended) => None,
            None => Some(DevicePushEvent::from_event(e, lead_min)),
            // Starts while snoozed (or already started): leave it out.
            Some(until) if e.start < until => None,
            // Starts after: the reminder may not ring before `until`.
            Some(until) => {
                let room = ((e.start - until) / 60).clamp(0, u8::MAX as i64) as u8;
                Some(DevicePushEvent::from_event(e, lead_min.min(room)))
            }
        })
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, start: i64) -> CalendarEvent {
        CalendarEvent {
            id: id.into(),
            title: id.into(),
            start,
            end: start + 1800,
            source: "ics".into(),
            is_all_day: false,
            is_tentative: false,
        }
    }

    #[test]
    fn snooze_holds_back_meetings_that_start_inside_it() {
        let until = 10_000;
        let events = [
            ev("now", until - 4000),    // in progress
            ev("inside", until - 600),  // starts during the snooze
            ev("edge", until + 2 * 60), // 2 minutes after it ends
            ev("later", until + 3600),  // well after
            ev("latest", until + 2 * 3600),
        ];
        let pushed = push_events(&events, 5, Some(until), None, 2);
        let got: Vec<(&str, u8)> = pushed
            .iter()
            .map(|e| (e.id.as_str(), e.chime_lead_min))
            .collect();
        // "edge" keeps only a 2-minute reminder so it rings after the snooze.
        assert_eq!(got, [("edge", 2), ("later", 5)]);
    }

    #[test]
    fn without_snooze_everything_goes_with_the_normal_lead() {
        let events = [ev("a", 100), ev("b", 200), ev("c", 300)];
        let pushed = push_events(&events, 10, None, None, 2);
        assert_eq!(pushed.len(), 2);
        assert!(pushed.iter().all(|e| e.chime_lead_min == 10));
        assert_eq!(pushed[0].id, "a");
    }

    #[test]
    fn meetings_that_started_during_a_snooze_stay_off_after_it() {
        let ended = 10_000;
        let events = [
            ev("started-during", ended - 600), // still running at resume
            ev("next", ended + 600),
        ];
        let pushed = push_events(&events, 5, None, Some(ended), 5);
        let ids: Vec<&str> = pushed.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["next"]);
    }

    #[test]
    fn clock_follows_the_locale_format() {
        use chrono::TimeZone;
        let t = chrono::Local
            .with_ymd_and_hms(2026, 11, 12, 14, 5, 0)
            .unwrap();
        // (Not via set_clock_format: that touches the tray, which a test
        // binary on Windows can't load without the app manifest.)
        TWELVE_HOUR.store(true, Ordering::Relaxed);
        assert_eq!(format_clock(t), "2:05 PM");
        TWELVE_HOUR.store(false, Ordering::Relaxed);
        assert_eq!(format_clock(t), "14:05");
    }
}
