use super::CalendarEvent;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::time::Duration;

/// Normalises a calendar feed URL: `webcal://` (what Apple Calendar and
/// others hand out) becomes `https://`; only http(s) feeds are accepted.
pub fn normalize_feed_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let rewritten = match trimmed.get(..9) {
        Some(prefix) if prefix.eq_ignore_ascii_case("webcal://") => {
            format!("https://{}", &trimmed[9..])
        }
        _ => trimmed.to_string(),
    };
    let parsed = url::Url::parse(&rewritten).map_err(|_| {
        "That doesn't look like a calendar link (expected https://... or webcal://...).".to_string()
    })?;
    match parsed.scheme() {
        "https" | "http" if parsed.host_str().is_some() => Ok(parsed.to_string()),
        _ => Err("Calendar links must start with https://, http:// or webcal://.".to_string()),
    }
}

pub async fn fetch_ics_events(ics_url: &str) -> Result<Vec<CalendarEvent>, String> {
    let ics_url = normalize_feed_url(ics_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    // Private feed URLs embed a secret token, so keep them out of error text.
    let res = client
        .get(&ics_url)
        .header("User-Agent", "CubiPal-Desktop/0.1.0")
        .send()
        .await
        .map_err(|e| format!("Could not download calendar feed: {}", e.without_url()))?;

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
    let mut is_cancelled = false;

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
            is_cancelled = false;
        } else if trimmed == "END:VEVENT" {
            if in_event && current_start > 0 && !is_cancelled {
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
            } else if key_upper.starts_with("STATUS") {
                if val.eq_ignore_ascii_case("TENTATIVE") {
                    is_tentative = true;
                } else if val.eq_ignore_ascii_case("CANCELLED") {
                    is_cancelled = true;
                }
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

/// RFC 5545 TEXT unescape in a single left-to-right pass, so an escaped
/// backslash followed by `n` (`\\n`) stays a backslash plus `n`.
fn unescape_ics_text(val: &str) -> String {
    let mut out = String::with_capacity(val.len());
    let mut chars = val.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other @ ('\\' | ',' | ';')) => out.push(other),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_is_single_pass() {
        assert_eq!(unescape_ics_text("a\\,b\\;c"), "a,b;c");
        assert_eq!(unescape_ics_text("line1\\nline2"), "line1\nline2");
        assert_eq!(unescape_ics_text("line1\\Nline2"), "line1\nline2");
        assert_eq!(unescape_ics_text("C:\\\\new"), "C:\\new");
        assert_eq!(unescape_ics_text("trailing\\"), "trailing\\");
        assert_eq!(unescape_ics_text("odd\\x"), "odd\\x");
    }

    #[test]
    fn webcal_becomes_https() {
        assert_eq!(
            normalize_feed_url(" webcal://p01-caldav.icloud.com/published/2/abc ").unwrap(),
            "https://p01-caldav.icloud.com/published/2/abc"
        );
        assert_eq!(
            normalize_feed_url("WEBCAL://example.com/cal.ics").unwrap(),
            "https://example.com/cal.ics"
        );
        assert_eq!(
            normalize_feed_url("https://example.com/cal.ics?token=x").unwrap(),
            "https://example.com/cal.ics?token=x"
        );
        assert!(normalize_feed_url("http://example.com/cal.ics").is_ok());
        assert!(normalize_feed_url("file:///etc/passwd").is_err());
        assert!(normalize_feed_url("ftp://example.com/cal.ics").is_err());
        assert!(normalize_feed_url("not a url").is_err());
    }

    fn vevent(extra: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Stand-up\\, daily\r\nDTSTART:20261101T150000Z\r\nDTEND:20261101T153000Z\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    #[test]
    fn parses_basic_event() {
        let events = parse_ics_content(&vevent("")).unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.id, "1");
        assert_eq!(e.title, "Stand-up, daily");
        assert_eq!(e.start, 1_793_545_200);
        assert_eq!(e.end, 1_793_547_000);
        assert!(!e.is_tentative && !e.is_all_day);
    }

    #[test]
    fn cancelled_events_are_skipped() {
        assert!(
            parse_ics_content(&vevent("STATUS:CANCELLED\r\n"))
                .unwrap()
                .is_empty()
        );
        let events = parse_ics_content(&vevent("STATUS:TENTATIVE\r\n")).unwrap();
        assert!(events[0].is_tentative);
        assert_eq!(
            parse_ics_content(&vevent("STATUS:CONFIRMED\r\n"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn folded_lines_and_all_day() {
        let raw = "BEGIN:VEVENT\nSUMMARY:Long\n  title\nDTSTART;VALUE=DATE:20261101\nEND:VEVENT\n";
        let events = parse_ics_content(raw).unwrap();
        assert_eq!(events[0].title, "Long title");
        assert!(events[0].is_all_day);
        assert_eq!(events[0].start, 1_793_491_200);
        assert_eq!(events[0].end, 1_793_491_200 + 1800);
        assert_eq!(events[0].id, "ics_1793491200");
    }

    #[test]
    fn datetime_parsing() {
        assert_eq!(
            parse_ics_datetime("20261101T150000Z"),
            Some((1_793_545_200, false))
        );
        assert_eq!(parse_ics_datetime("20261101"), Some((1_793_491_200, true)));
        assert_eq!(parse_ics_datetime("2026110"), None);
        assert_eq!(parse_ics_datetime("garbage"), None);
        assert_eq!(parse_ics_datetime("20261301T000000Z"), None);
    }
}
