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
//! Snooze lasts until it expires or "Resume Alerts" is chosen; it is not
//! kept across restarts.

use crate::calendar::{CalendarEvent, DevicePushEvent};
use chrono::Utc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::AppHandle;

static SNOOZE_UNTIL: Mutex<Option<i64>> = Mutex::new(None);
/// Bumped on every change, so an old expiry timer can't end a newer snooze.
static GENERATION: AtomicU64 = AtomicU64::new(0);

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

pub fn snooze(app: &AppHandle, minutes: i64) {
    let until = Utc::now().timestamp() + minutes * 60;
    if let Ok(mut g) = SNOOZE_UNTIL.lock() {
        *g = Some(until);
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    crate::tray::show_snooze(Some(until));
    crate::calendar::trigger_immediate_sync();

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs((minutes * 60).max(0) as u64)).await;
        if GENERATION.load(Ordering::SeqCst) == generation {
            resume(&app);
        }
    });
}

pub fn resume(_app: &AppHandle) {
    if let Ok(mut g) = SNOOZE_UNTIL.lock() {
        *g = None;
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
    crate::tray::show_snooze(None);
    crate::calendar::trigger_immediate_sync();
}

/// The (at most `max`) events to push to the Pal, honouring a snooze that
/// ends at `snooze_until`.
pub fn push_events(
    events: &[CalendarEvent],
    lead_min: u8,
    snooze_until: Option<i64>,
    max: usize,
) -> Vec<DevicePushEvent> {
    events
        .iter()
        .filter_map(|e| match snooze_until {
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
        let pushed = push_events(&events, 5, Some(until), 2);
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
        let pushed = push_events(&events, 10, None, 2);
        assert_eq!(pushed.len(), 2);
        assert!(pushed.iter().all(|e| e.chime_lead_min == 10));
        assert_eq!(pushed[0].id, "a");
    }
}
