use super::CalendarEvent;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::time::Duration;

pub async fn fetch_ics_events(ics_url: &str) -> Result<Vec<CalendarEvent>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(ics_url)
        .header("User-Agent", "CubiPal-Desktop/0.1.0")
        .send()
        .await
        .map_err(|e| format!("Could not download calendar from {}: {}", ics_url, e))?;

    if !res.status().is_success() {
        return Err(format!("Server returned HTTP {}", res.status()));
    }

    let text = res
        .text()
        .await
        .map_err(|e| format!("Failed to read calendar body: {}", e))?;

    parse_ics_content(&text)
}

pub fn parse_ics_content(raw: &str) -> Result<Vec<CalendarEvent>, String> {
    // 1. Unfold lines per RFC 5545 (a newline followed by a space or tab is a continuation)
    let mut unfolded = String::with_capacity(raw.len());
    for line in raw.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            unfolded.push_str(&line[1..]);
        } else {
            if !unfolded.is_empty() {
                unfolded.push('\n');
            }
            unfolded.push_str(line);
        }
    }

    let mut events = Vec::new();
    let mut in_event = false;
    let mut current_id = String::new();
    let mut current_title = String::new();
    let mut current_start = 0i64;
    let mut current_end = 0i64;
    let mut is_all_day = false;
    let mut is_tentative = false;

    for line in unfolded.lines() {
        let trimmed = line.trim();
        if trimmed == "BEGIN:VEVENT" {
            in_event = true;
            current_id.clear();
            current_title = "(Untitled Meeting)".to_string();
            current_start = 0;
            current_end = 0;
            is_all_day = false;
            is_tentative = false;
        } else if trimmed == "END:VEVENT" {
            if in_event && current_start > 0 {
                if current_end <= current_start {
                    current_end = current_start + 1800; // default 30 min
                }
                events.push(CalendarEvent {
                    id: if current_id.is_empty() {
                        format!("ics_{}", current_start)
                    } else {
                        current_id.clone()
                    },
                    title: current_title.clone(),
                    start: current_start,
                    end: current_end,
                    source: "ics".to_string(),
                    is_all_day,
                    is_tentative,
                });
            }
            in_event = false;
        } else if in_event && let Some(pos) = trimmed.find(':') {
            let (key_part, val_part) = trimmed.split_at(pos);
            let val = &val_part[1..];
            let key_upper = key_part.to_uppercase();

            if key_upper == "UID" {
                current_id = val.to_string();
            } else if key_upper.starts_with("SUMMARY") {
                current_title = unescape_ics_text(val);
            } else if key_upper.starts_with("DTSTART") {
                if let Some((epoch, all_day)) = parse_ics_datetime(val) {
                    current_start = epoch;
                    is_all_day = all_day;
                }
            } else if key_upper.starts_with("DTEND") {
                if let Some((epoch, _)) = parse_ics_datetime(val) {
                    current_end = epoch;
                }
            } else if key_upper.starts_with("STATUS") && val.eq_ignore_ascii_case("TENTATIVE") {
                is_tentative = true;
            }
        }
    }

    Ok(events)
}

fn parse_ics_datetime(val: &str) -> Option<(i64, bool)> {
    let clean = val.trim();

    // Case 1: UTC datetime ending in Z: "20261101T150000Z"
    if clean.ends_with('Z')
        && let Ok(dt) = NaiveDateTime::parse_from_str(&clean[..clean.len() - 1], "%Y%m%dT%H%M%S")
    {
        return Some((Utc.from_utc_datetime(&dt).timestamp(), false));
    }

    // Case 2: Local datetime: "20261101T150000"
    if clean.contains('T')
        && let Ok(dt) = NaiveDateTime::parse_from_str(clean, "%Y%m%dT%H%M%S")
    {
        // Interpret as UTC timestamp fallback
        return Some((Utc.from_utc_datetime(&dt).timestamp(), false));
    }

    // Case 3: Date only (all day): "20261101"
    if clean.len() == 8
        && let Ok(d) = NaiveDate::parse_from_str(clean, "%Y%m%d")
        && let Some(dt) = d.and_hms_opt(0, 0, 0)
    {
        return Some((Utc.from_utc_datetime(&dt).timestamp(), true));
    }

    None
}

fn unescape_ics_text(val: &str) -> String {
    val.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}
