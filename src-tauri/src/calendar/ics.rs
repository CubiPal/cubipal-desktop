//! ICS (iCalendar, RFC 5545) feeds: parsing, time zones and recurrence.

use super::CalendarEvent;
use super::tz::resolve_tzid;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rrule::{RRule, Tz as RTz, Unvalidated};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Events are synced for this far ahead, as for Google and Microsoft.
pub const SYNC_WINDOW_HOURS: i64 = 24;
/// Most occurrences taken from one recurring series.
const MAX_INSTANCES_PER_SERIES: u16 = 50;
/// Most events taken from one feed.
const MAX_EVENTS_PER_FEED: usize = 500;
/// A timed event without DTEND or DURATION is shown as 30 minutes long.
const DEFAULT_TIMED_SECS: i64 = 30 * 60;
/// Largest feed the app downloads. Real calendars are far smaller.
const MAX_FEED_BYTES: usize = 5 * 1024 * 1024;
/// Most VEVENTs read from one feed; the rest are ignored.
const MAX_VEVENTS_PER_FEED: usize = 5000;
/// Most occurrences walked through for one series (counted from its start,
/// after any fast-forward), however few of them fall in the window.
const MAX_ITERATIONS_PER_SERIES: usize = 20_000;
/// Time one feed may spend expanding recurrences. Series reached after that
/// keep only their first occurrence.
const EXPANSION_BUDGET: Duration = Duration::from_secs(2);

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
    let text = fetch_feed_text(ics_url, MAX_FEED_BYTES).await?;
    let now = Utc::now();
    // Parsing and recurrence expansion are CPU work: keep them off the async
    // runtime's worker threads.
    tokio::task::spawn_blocking(move || {
        parse_ics_events(&text, now, ChronoDuration::hours(SYNC_WINDOW_HOURS))
    })
    .await
    .map_err(|e| format!("Couldn't read the calendar feed: {}", e))
}

/// Downloads a feed, refusing a body over `max_bytes`.
async fn fetch_feed_text(ics_url: &str, max_bytes: usize) -> Result<String, String> {
    let ics_url = normalize_feed_url(ics_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("CubiPal-Desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    // Private feed URLs embed a secret token, so keep them out of error text.
    let mut res = client.get(&ics_url).send().await.map_err(feed_error)?;

    if !res.status().is_success() {
        return Err(format!(
            "The calendar server answered with an error (HTTP {}). Check the link.",
            res.status().as_u16()
        ));
    }

    let too_big = || {
        format!(
            "The calendar feed is larger than {} MB, so it wasn't read.",
            max_bytes.div_ceil(1024 * 1024)
        )
    };
    if res
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(feed_error)? {
        if body.len() + chunk.len() > max_bytes {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Plain-language text for a failed feed download (never includes the URL).
fn feed_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "The calendar server took too long to answer. Try again later.".to_string()
    } else if e.is_connect() || e.is_request() {
        "Couldn't connect to the calendar server. Check your internet connection and the link."
            .to_string()
    } else {
        format!("Couldn't download the calendar feed: {}", e.without_url())
    }
}

// ---------------------------------------------------------------------------
// Content lines
// ---------------------------------------------------------------------------

/// One `NAME;PARAM=VALUE:value` line.
#[derive(Debug, Clone, Default)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Splits a content line, honouring quoted parameter values (which may
/// contain `:` and `;`).
fn parse_content_line(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);

    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (i, c) in head.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(&head[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&head[start..]);

    let name = parts.first()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts[1..]
        .iter()
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            )
        })
        .collect();
    Some(Prop {
        name,
        params,
        value: value.trim().to_string(),
    })
}

/// Unfolds lines per RFC 5545 (a line starting with a space or tab
/// continues the previous one).
fn unfold(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// VEVENTs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct RawEvent {
    uid: String,
    summary: Option<String>,
    status: Option<String>,
    dtstart: Option<Prop>,
    dtend: Option<Prop>,
    duration: Option<String>,
    rrule: Option<String>,
    exdates: Vec<Prop>,
    rdates: Vec<Prop>,
    recurrence_id: Option<Prop>,
}

impl RawEvent {
    fn is_cancelled(&self) -> bool {
        self.status
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED"))
    }

    fn is_tentative(&self) -> bool {
        self.status
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("TENTATIVE"))
    }

    fn title(&self) -> String {
        self.summary
            .clone()
            .unwrap_or_else(|| "(Untitled Meeting)".to_string())
    }
}

/// Collects the VEVENTs of a feed. Properties of components nested in an
/// event (VALARM) are ignored, so an alarm's SUMMARY or DURATION can't
/// replace the event's.
fn collect_events(raw: &str) -> Vec<RawEvent> {
    let mut events = Vec::new();
    let mut current: Option<RawEvent> = None;
    let mut nested = 0usize;

    for line in unfold(raw) {
        let Some(prop) = parse_content_line(line.trim_end()) else {
            continue;
        };
        match prop.name.as_str() {
            "BEGIN" if prop.value.eq_ignore_ascii_case("VEVENT") && current.is_none() => {
                current = Some(RawEvent::default());
                nested = 0;
            }
            "BEGIN" if current.is_some() => nested += 1,
            "END" if current.is_some() && nested > 0 => nested -= 1,
            "END" if prop.value.eq_ignore_ascii_case("VEVENT") => {
                if let Some(ev) = current.take() {
                    events.push(ev);
                    if events.len() >= MAX_VEVENTS_PER_FEED {
                        break;
                    }
                }
            }
            _ if nested > 0 => {}
            _ => {
                let Some(ev) = current.as_mut() else { continue };
                match prop.name.as_str() {
                    "UID" => ev.uid = prop.value,
                    "SUMMARY" => ev.summary = Some(unescape_ics_text(&prop.value)),
                    "STATUS" => ev.status = Some(prop.value),
                    "DTSTART" => ev.dtstart = Some(prop),
                    "DTEND" => ev.dtend = Some(prop),
                    "DURATION" => ev.duration = Some(prop.value),
                    "RRULE" => ev.rrule = Some(prop.value),
                    "EXDATE" => ev.exdates.push(prop),
                    "RDATE" => ev.rdates.push(prop),
                    "RECURRENCE-ID" => ev.recurrence_id = Some(prop),
                    _ => {}
                }
            }
        }
    }
    events
}

// ---------------------------------------------------------------------------
// Date-times
// ---------------------------------------------------------------------------

/// The zone a DATE-TIME is in: UTC (`Z`), its `TZID`, or local time for
/// floating times and unknown zone names.
fn zone_for(prop: &Prop, value: &str) -> RTz {
    if value.ends_with(['Z', 'z']) {
        return RTz::UTC;
    }
    match prop.param("TZID").and_then(resolve_tzid) {
        Some(tz) => RTz::Tz(tz),
        None => RTz::LOCAL,
    }
}

/// Midnight (or the first valid time after it) on `date` in `tz`.
fn start_of_day(date: NaiveDate, tz: &RTz) -> Option<DateTime<RTz>> {
    (0..3)
        .filter_map(|h| date.and_hms_opt(h, 0, 0))
        .find_map(|naive| tz.from_local_datetime(&naive).earliest())
}

/// A wall-clock time in `tz`; a time skipped by a DST change moves forward
/// an hour.
fn local_in(naive: NaiveDateTime, tz: &RTz) -> Option<DateTime<RTz>> {
    tz.from_local_datetime(&naive).earliest().or_else(|| {
        tz.from_local_datetime(&(naive + ChronoDuration::hours(1)))
            .earliest()
    })
}

/// Parses one DATE or DATE-TIME value of `prop`. Returns the instant and
/// whether it is a whole day (`VALUE=DATE`, local midnight).
fn parse_value(prop: &Prop, value: &str) -> Option<(DateTime<RTz>, bool)> {
    let v = value.trim();
    let is_date = prop
        .param("VALUE")
        .is_some_and(|x| x.eq_ignore_ascii_case("DATE"))
        || (v.len() == 8 && v.chars().all(|c| c.is_ascii_digit()));
    if is_date {
        let date = NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()?;
        return start_of_day(date, &RTz::LOCAL).map(|d| (d, true));
    }
    let tz = zone_for(prop, v);
    let naive =
        NaiveDateTime::parse_from_str(v.trim_end_matches(['Z', 'z']), "%Y%m%dT%H%M%S").ok()?;
    local_in(naive, &tz).map(|d| (d, false))
}

/// All values of a list property (`EXDATE:20261101T090000,20261108T090000`).
fn parse_list(prop: &Prop) -> Vec<DateTime<RTz>> {
    prop.value
        .split(',')
        .filter_map(|v| parse_value(prop, v).map(|(d, _)| d))
        .collect()
}

/// RFC 5545 DURATION (`PT1H30M`, `P1D`, `P2W`, `-PT5M`), in seconds.
fn parse_duration(raw: &str) -> Option<i64> {
    let s = raw.trim();
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut seen_unit = false;
    for c in s.chars() {
        match c.to_ascii_uppercase() {
            'T' => {}
            d if d.is_ascii_digit() => num.push(d),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match unit {
                    'W' => 7 * 86_400,
                    'D' => 86_400,
                    'H' => 3_600,
                    'M' => 60,
                    'S' => 1,
                    _ => return None,
                };
                seen_unit = true;
            }
        }
    }
    (seen_unit && num.is_empty()).then_some(sign * total)
}

/// How long the event lasts: DTEND - DTSTART, else DURATION, else a day for
/// all-day events and 30 minutes for timed ones.
fn event_length(ev: &RawEvent, start: &DateTime<RTz>, all_day: bool) -> i64 {
    let from_end = ev
        .dtend
        .as_ref()
        .and_then(|p| parse_value(p, &p.value))
        .map(|(end, _)| end.timestamp() - start.timestamp());
    let length = from_end.or_else(|| ev.duration.as_deref().and_then(parse_duration));
    match length {
        Some(secs) if secs > 0 => secs,
        _ if all_day => 86_400,
        _ => DEFAULT_TIMED_SECS,
    }
}

// ---------------------------------------------------------------------------
// Recurrence
// ---------------------------------------------------------------------------

/// rrule requires UNTIL in UTC when DTSTART has a zone; some feeds write a
/// local UNTIL. Rewrites it as UTC in the event's zone.
fn normalize_until(rule: &str, tz: &RTz) -> String {
    if !matches!(tz, RTz::Tz(_)) {
        return rule.to_string();
    }
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("UNTIL") && !v.ends_with(['Z', 'z']) => {
                let v = v.trim();
                let naive = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S")
                    .ok()
                    .or_else(|| {
                        NaiveDate::parse_from_str(v, "%Y%m%d")
                            .ok()
                            .and_then(|d| d.and_hms_opt(23, 59, 59))
                    });
                match naive.and_then(|n| local_in(n, tz)) {
                    Some(dt) => {
                        format!("UNTIL={}", dt.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ"))
                    }
                    None => part.to_string(),
                }
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The `KEY=VALUE` parts of an RRULE, keys in upper case.
fn rule_parts(rule: &str) -> Vec<(String, String)> {
    rule.split(';')
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_string()))
        .collect()
}

fn rule_value<'a>(parts: &'a [(String, String)], key: &str) -> Option<&'a str> {
    parts
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[derive(Debug)]
enum FastForward {
    /// Keep the rule and start as they are.
    Unchanged,
    /// Start the series here instead, with this (COUNT-adjusted) rule.
    Moved { rule: String, start: DateTime<RTz> },
    /// A COUNT series that ended before the window.
    Finished,
}

/// Moves the start of an old HOURLY, DAILY or WEEKLY series forward by a
/// whole number of periods (keeping its wall-clock time and weekday) to
/// about a day before `earliest`, so expansion doesn't walk through years of
/// past occurrences.
///
/// Whole periods keep every BYxxx pattern aligned. COUNT is reduced by the
/// periods skipped, which is only exact when each period has exactly one
/// occurrence, so a COUNT rule with BYxxx parts isn't moved (the iteration
/// cap bounds it instead). BYSETPOS rules are never moved.
fn fast_forward(
    parts: &[(String, String)],
    start: DateTime<RTz>,
    earliest: DateTime<Utc>,
) -> FastForward {
    let unit_secs: i64 = match rule_value(parts, "FREQ").map(str::to_ascii_uppercase) {
        Some(f) if f == "HOURLY" => 3_600,
        Some(f) if f == "DAILY" => 86_400,
        Some(f) if f == "WEEKLY" => 7 * 86_400,
        _ => return FastForward::Unchanged,
    };
    if rule_value(parts, "BYSETPOS").is_some() {
        return FastForward::Unchanged;
    }
    let count = match rule_value(parts, "COUNT") {
        Some(c) => match c.parse::<i64>() {
            Ok(c) if c > 0 => Some(c),
            _ => return FastForward::Unchanged,
        },
        None => None,
    };
    if count.is_some() && parts.iter().any(|(k, _)| k.starts_with("BY")) {
        return FastForward::Unchanged;
    }
    let interval = match rule_value(parts, "INTERVAL") {
        Some(i) => match i.parse::<i64>() {
            Ok(i) if (1..=10_000).contains(&i) => i,
            _ => return FastForward::Unchanged,
        },
        None => 1,
    };
    let period = unit_secs * interval;

    let tz = start.timezone();
    // A day early, so a DST change or the zone offset can't skip one.
    let target = (earliest - ChronoDuration::days(1))
        .with_timezone(&tz)
        .naive_local();
    let base = start.naive_local();
    let mut k = (target - base).num_seconds() / period;
    if k < 1 {
        return FastForward::Unchanged;
    }
    // Occurrences 0..k all start before `target`.
    if count.is_some_and(|c| c <= k) {
        return FastForward::Finished;
    }
    // If the new start's wall-clock time doesn't exist that day (skipped by
    // a DST change), start a period earlier instead.
    for _ in 0..3 {
        if k < 1 {
            break;
        }
        let naive = base + ChronoDuration::seconds(k * period);
        if let Some(new_start) = tz.from_local_datetime(&naive).earliest() {
            let rule = parts
                .iter()
                .map(|(key, v)| match (key.as_str(), count) {
                    ("COUNT", Some(c)) => format!("COUNT={}", c - k),
                    _ => format!("{}={}", key, v),
                })
                .collect::<Vec<_>>()
                .join(";");
            return FastForward::Moved {
                rule,
                start: new_start,
            };
        }
        k -= 1;
    }
    FastForward::Unchanged
}

/// Occurrence starts of a recurring event that may overlap
/// `[from, until)`: those starting in `[from - length, until)`. `None` when
/// the rule can't be understood or is refused (the caller then keeps only
/// DTSTART).
///
/// The work is bounded (B-1): SECONDLY and MINUTELY rules are refused (no
/// meeting calendar needs them), old series are fast-forwarded to near the
/// window, at most [`MAX_ITERATIONS_PER_SERIES`] occurrences are walked, and
/// the walk stops at `deadline`.
fn expand_rrule(
    rule: &str,
    start: DateTime<RTz>,
    length: i64,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
    deadline: Instant,
) -> Option<Vec<DateTime<RTz>>> {
    let rule = normalize_until(rule, &start.timezone());
    let parts = rule_parts(&rule);
    let freq = rule_value(&parts, "FREQ")?.to_ascii_uppercase();
    if matches!(freq.as_str(), "SECONDLY" | "MINUTELY") {
        return None;
    }
    let earliest = from - ChronoDuration::seconds(length);
    let (rule, start) = match fast_forward(&parts, start, earliest) {
        FastForward::Unchanged => (rule, start),
        FastForward::Moved { rule, start } => (rule, start),
        FastForward::Finished => return Some(Vec::new()),
    };

    let parsed: RRule<Unvalidated> = rule.parse().ok()?;
    // `limit()` makes rrule give up on a rule that never matches (after
    // 100 000 empty periods) instead of looping forever.
    let set = parsed.build(start).ok()?.limit();
    let after = earliest.with_timezone(&RTz::UTC);
    let before = until.with_timezone(&RTz::UTC);
    let mut out = Vec::new();
    for (i, occ) in (&set).into_iter().enumerate() {
        if i >= MAX_ITERATIONS_PER_SERIES
            || occ > before
            || (i % 256 == 255 && Instant::now() > deadline)
        {
            break;
        }
        if occ >= after {
            out.push(occ);
            if out.len() >= MAX_INSTANCES_PER_SERIES as usize {
                break;
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Feed -> events
// ---------------------------------------------------------------------------

/// Events of a feed that overlap `[now, now + window)`. Recurring events
/// are expanded (RRULE, RDATE, minus EXDATE), and RECURRENCE-ID overrides
/// replace or cancel single occurrences.
pub fn parse_ics_events(
    raw: &str,
    now: DateTime<Utc>,
    window: ChronoDuration,
) -> Vec<CalendarEvent> {
    let window_end = now + window;
    let deadline = Instant::now() + EXPANSION_BUDGET;
    let raw_events = collect_events(raw);

    // Overrides of single occurrences, keyed by (UID, original start).
    let mut overridden: HashSet<(String, i64)> = HashSet::new();
    let mut overrides = Vec::new();
    let mut masters = Vec::new();
    for ev in raw_events {
        match ev
            .recurrence_id
            .as_ref()
            .and_then(|p| parse_value(p, &p.value))
        {
            Some((rid, _)) => {
                overridden.insert((ev.uid.clone(), rid.timestamp()));
                overrides.push((rid.timestamp(), ev));
            }
            None => masters.push(ev),
        }
    }

    let mut out: Vec<CalendarEvent> = Vec::new();
    let mut push = |id: String, ev: &RawEvent, start: i64, length: i64, all_day: bool| {
        let end = start + length;
        if end > now.timestamp() && start < window_end.timestamp() {
            out.push(CalendarEvent {
                id,
                title: ev.title(),
                start,
                end,
                source: "ics".to_string(),
                is_all_day: all_day,
                is_tentative: ev.is_tentative(),
            });
        }
    };

    for ev in &masters {
        if ev.is_cancelled() {
            continue;
        }
        let Some((start, all_day)) = ev.dtstart.as_ref().and_then(|p| parse_value(p, &p.value))
        else {
            continue;
        };
        let length = event_length(ev, &start, all_day);
        let recurring = ev.rrule.is_some() || !ev.rdates.is_empty();
        if !recurring {
            let id = if ev.uid.is_empty() {
                format!("ics_{}", start.timestamp())
            } else {
                ev.uid.clone()
            };
            push(id, ev, start.timestamp(), length, all_day);
            continue;
        }

        let mut starts: Vec<DateTime<RTz>> = match ev.rrule.as_deref() {
            Some(rule) if Instant::now() < deadline => {
                expand_rrule(rule, start, length, now, window_end, deadline)
                    .unwrap_or_else(|| vec![start])
            }
            _ => vec![start],
        };
        starts.extend(ev.rdates.iter().flat_map(parse_list));
        let excluded: HashSet<i64> = ev
            .exdates
            .iter()
            .flat_map(parse_list)
            .map(|d| d.timestamp())
            .collect();

        let mut seen = HashSet::new();
        for occ in starts
            .iter()
            .map(|d| d.timestamp())
            .take(MAX_INSTANCES_PER_SERIES as usize)
        {
            if !seen.insert(occ)
                || excluded.contains(&occ)
                || overridden.contains(&(ev.uid.clone(), occ))
            {
                continue;
            }
            push(format!("{}_{}", ev.uid, occ), ev, occ, length, all_day);
        }
    }

    // Moved or edited single occurrences (a cancelled one just disappears).
    for (rid, ev) in &overrides {
        if ev.is_cancelled() {
            continue;
        }
        let Some((start, all_day)) = ev.dtstart.as_ref().and_then(|p| parse_value(p, &p.value))
        else {
            continue;
        };
        let length = event_length(ev, &start, all_day);
        push(
            format!("{}_{}", ev.uid, rid),
            ev,
            start.timestamp(),
            length,
            all_day,
        );
    }

    out.sort_by_key(|e| e.start);
    out.truncate(MAX_EVENTS_PER_FEED);
    out
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
    use chrono::Local;
    use std::collections::HashMap;

    fn by_id(events: &[CalendarEvent]) -> HashMap<&str, &CalendarEvent> {
        events.iter().map(|e| (e.id.as_str(), e)).collect()
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        utc(y, mo, d, h, mi).timestamp()
    }

    fn cal(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{body}END:VCALENDAR\r\n")
    }

    fn parse(raw: &str, now: DateTime<Utc>, days: i64) -> Vec<CalendarEvent> {
        parse_ics_events(raw, now, ChronoDuration::days(days))
    }

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
        cal(&format!(
            "BEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Stand-up\\, daily\r\nDTSTART:20261101T150000Z\r\nDTEND:20261101T153000Z\r\n{extra}END:VEVENT\r\n"
        ))
    }

    #[test]
    fn parses_basic_utc_event() {
        let events = parse(&vevent(""), utc(2026, 11, 1, 14, 0), 1);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.id, "1");
        assert_eq!(e.title, "Stand-up, daily");
        assert_eq!(e.start, 1_793_545_200);
        assert_eq!(e.end, 1_793_547_000);
        assert!(!e.is_tentative && !e.is_all_day);
        // Outside the window: already over, or too far ahead.
        assert!(parse(&vevent(""), utc(2026, 11, 1, 16, 0), 1).is_empty());
        assert!(parse(&vevent(""), utc(2026, 10, 30, 0, 0), 1).is_empty());
    }

    #[test]
    fn cancelled_and_tentative() {
        let now = utc(2026, 11, 1, 14, 0);
        assert!(parse(&vevent("STATUS:CANCELLED\r\n"), now, 1).is_empty());
        assert!(parse(&vevent("STATUS:TENTATIVE\r\n"), now, 1)[0].is_tentative);
        assert_eq!(parse(&vevent("STATUS:CONFIRMED\r\n"), now, 1).len(), 1);
    }

    #[test]
    fn alarm_properties_do_not_leak_into_the_event() {
        let raw = vevent(
            "BEGIN:VALARM\r\nACTION:EMAIL\r\nSUMMARY:Alarm mail\r\nDURATION:PT10H\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\n",
        );
        let events = parse(&raw, utc(2026, 11, 1, 14, 0), 1);
        assert_eq!(events[0].title, "Stand-up, daily");
        assert_eq!(events[0].end - events[0].start, 1800);
    }

    #[test]
    fn tzid_iana_and_windows_names() {
        for tzid in [
            "America/New_York",
            "\"America/New_York\"",
            "Eastern Standard Time",
            "/mozilla.org/20050126_1/America/New_York",
        ] {
            let raw = cal(&format!(
                "BEGIN:VEVENT\r\nUID:tz\r\nDTSTART;TZID={tzid}:20261112T090000\r\nDTEND;TZID={tzid}:20261112T100000\r\nEND:VEVENT\r\n"
            ));
            let e = &parse(&raw, utc(2026, 11, 12, 0, 0), 1)[0];
            // 09:00 EST is 14:00 UTC.
            assert_eq!(e.start, ts(2026, 11, 12, 14, 0), "TZID={tzid}");
            assert_eq!(e.end, ts(2026, 11, 12, 15, 0), "TZID={tzid}");
        }
        // Summer time in Europe: 09:00 CEST is 07:00 UTC.
        let raw = cal(
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART;TZID=W. Europe Standard Time:20260701T090000\r\nDURATION:PT45M\r\nEND:VEVENT\r\n",
        );
        let e = &parse(&raw, utc(2026, 7, 1, 0, 0), 1)[0];
        assert_eq!(e.start, ts(2026, 7, 1, 7, 0));
        assert_eq!(e.end, ts(2026, 7, 1, 7, 45));
    }

    #[test]
    fn floating_and_unknown_zone_times_are_local() {
        let naive = NaiveDate::from_ymd_opt(2026, 11, 12)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        let expected = Local
            .from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp();
        for dtstart in [
            "DTSTART:20261112T090000",
            "DTSTART;TZID=Mars/Olympus:20261112T090000",
        ] {
            let raw = cal(&format!(
                "BEGIN:VEVENT\r\nUID:f\r\n{dtstart}\r\nEND:VEVENT\r\n"
            ));
            let e = &parse(&raw, utc(2026, 11, 11, 0, 0), 3)[0];
            assert_eq!(e.start, expected, "{dtstart}");
            assert_eq!(e.end, expected + DEFAULT_TIMED_SECS);
        }
    }

    #[test]
    fn all_day_events_start_at_local_midnight() {
        let midnight = |d: u32| {
            Local
                .from_local_datetime(
                    &NaiveDate::from_ymd_opt(2026, 11, d)
                        .unwrap()
                        .and_hms_opt(0, 0, 0)
                        .unwrap(),
                )
                .earliest()
                .unwrap()
                .timestamp()
        };
        let raw = cal(
            "BEGIN:VEVENT\r\nUID:ad\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261112\r\nDTEND;VALUE=DATE:20261113\r\nEND:VEVENT\r\n",
        );
        let e = &parse(&raw, utc(2026, 11, 11, 12, 0), 3)[0];
        assert!(e.is_all_day);
        assert_eq!(e.start, midnight(12));
        assert_eq!(e.end, midnight(13));
        // No DTEND: one day. A bare 8-digit value is a date too.
        let raw = cal("BEGIN:VEVENT\r\nUID:ad2\r\nDTSTART:20261112\r\nEND:VEVENT\r\n");
        let e = &parse(&raw, utc(2026, 11, 11, 12, 0), 3)[0];
        assert!(e.is_all_day);
        assert_eq!(e.end - e.start, 86_400);
    }

    const WEEKLY: &str = "BEGIN:VEVENT\r\nUID:weekly\r\nSUMMARY:Team sync\r\nDTSTART;TZID=America/New_York:20260105T090000\r\nDTEND;TZID=America/New_York:20260105T093000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\nEXDATE;TZID=America/New_York:20261116T090000\r\nEND:VEVENT\r\n";

    #[test]
    fn weekly_rrule_with_exdate_and_dst() {
        // Nov 16 is excluded; Nov 23 and 30 remain (09:00 EST = 14:00 UTC).
        let events = parse(&cal(WEEKLY), utc(2026, 11, 15, 12, 0), 16);
        let starts: Vec<i64> = events.iter().map(|e| e.start).collect();
        assert_eq!(starts, [ts(2026, 11, 23, 14, 0), ts(2026, 11, 30, 14, 0)]);
        assert!(
            events
                .iter()
                .all(|e| e.end - e.start == 1800 && e.title == "Team sync")
        );
        assert_eq!(events[0].id, format!("weekly_{}", ts(2026, 11, 23, 14, 0)));
        // In July the same 09:00 is EDT = 13:00 UTC.
        let events = parse(&cal(WEEKLY), utc(2026, 7, 5, 0, 0), 2);
        assert_eq!(
            events.iter().map(|e| e.start).collect::<Vec<_>>(),
            [ts(2026, 7, 6, 13, 0)]
        );
        // A meeting in progress is kept.
        let events = parse(&cal(WEEKLY), utc(2026, 11, 23, 14, 10), 1);
        assert_eq!(events[0].start, ts(2026, 11, 23, 14, 0));
    }

    #[test]
    fn recurrence_id_overrides_move_and_cancel_occurrences() {
        let raw = cal(&format!(
            "{WEEKLY}BEGIN:VEVENT\r\nUID:weekly\r\nRECURRENCE-ID;TZID=America/New_York:20261123T090000\r\nSUMMARY:Team sync (moved)\r\nDTSTART;TZID=America/New_York:20261123T110000\r\nDTEND;TZID=America/New_York:20261123T120000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:weekly\r\nRECURRENCE-ID;TZID=America/New_York:20261130T090000\r\nSTATUS:CANCELLED\r\nDTSTART;TZID=America/New_York:20261130T090000\r\nEND:VEVENT\r\n"
        ));
        let events = parse(&raw, utc(2026, 11, 15, 12, 0), 23);
        let got: Vec<(i64, &str)> = events.iter().map(|e| (e.start, e.title.as_str())).collect();
        assert_eq!(
            got,
            [
                (ts(2026, 11, 23, 16, 0), "Team sync (moved)"),
                (ts(2026, 12, 7, 14, 0), "Team sync"),
            ]
        );
        let ids = by_id(&events);
        let moved = ids[format!("weekly_{}", ts(2026, 11, 23, 14, 0)).as_str()];
        assert_eq!(moved.end - moved.start, 3600);
    }

    #[test]
    fn local_until_is_accepted_and_instances_are_capped() {
        // UNTIL in local time (not UTC) with a TZID start, as some apps write it.
        let raw = cal(
            "BEGIN:VEVENT\r\nUID:u\r\nDTSTART;TZID=Europe/London:20261101T100000\r\nRRULE:FREQ=DAILY;UNTIL=20261103T100000\r\nEND:VEVENT\r\n",
        );
        let events = parse(&raw, utc(2026, 11, 1, 0, 0), 10);
        assert_eq!(events.len(), 3);

        let raw = cal(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART:20260101T080000Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n",
        );
        let events = parse(&raw, utc(2026, 11, 1, 0, 0), 365);
        assert_eq!(events.len(), MAX_INSTANCES_PER_SERIES as usize);
        assert_eq!(events[0].start, ts(2026, 11, 1, 8, 0));
    }

    #[test]
    fn bad_rules_fall_back_to_the_first_occurrence() {
        let raw = cal(
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261112T150000Z\r\nRRULE:FREQ=SOMETIMES\r\nEND:VEVENT\r\n",
        );
        let events = parse(&raw, utc(2026, 11, 12, 0, 0), 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start, ts(2026, 11, 12, 15, 0));
    }

    #[test]
    fn folded_lines_and_quoted_params() {
        let raw = "BEGIN:VEVENT\nUID:q\nSUMMARY:Long\n  title\nDTSTART;X-NOTE=\"a:b;c\";TZID=UTC:20261101T090000\nEND:VEVENT\n";
        let events = parse(raw, utc(2026, 11, 1, 0, 0), 1);
        assert_eq!(events[0].title, "Long title");
        assert_eq!(events[0].start, ts(2026, 11, 1, 9, 0));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1D"), Some(86_400));
        assert_eq!(parse_duration("P1W"), Some(604_800));
        assert_eq!(parse_duration("P1DT2H"), Some(93_600));
        assert_eq!(parse_duration("-PT5M"), Some(-300));
        assert_eq!(parse_duration("PT"), None);
        assert_eq!(parse_duration("1H"), None);
        assert_eq!(parse_duration("PT5"), None);
    }

    // -- B-1: bounded recurrence work -------------------------------------

    fn far_deadline() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    /// The same series expanded by rrule from its real start, with no
    /// fast-forward and no caps.
    fn reference(
        rule: &str,
        start: DateTime<RTz>,
        after: DateTime<Utc>,
        before: DateTime<Utc>,
    ) -> Vec<i64> {
        let parsed: RRule<Unvalidated> = rule.parse().unwrap();
        parsed
            .build(start)
            .unwrap()
            .after(after.with_timezone(&RTz::UTC))
            .before(before.with_timezone(&RTz::UTC))
            .all(u16::MAX)
            .dates
            .iter()
            .map(|d| d.timestamp())
            .take(MAX_INSTANCES_PER_SERIES as usize)
            .collect()
    }

    #[test]
    fn fast_forward_gives_the_same_occurrences_as_a_full_walk() {
        let now = utc(2026, 11, 12, 10, 0);
        let until = now + ChronoDuration::days(20);
        let ny = RTz::Tz(chrono_tz::America::New_York);
        let london = RTz::Tz(chrono_tz::Europe::London);
        let at = |tz: &RTz, y, mo, d, h, mi| tz.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap();
        let cases = [
            (
                "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE,FR",
                at(&ny, 2019, 3, 4, 9, 30),
            ),
            ("FREQ=WEEKLY", at(&ny, 2012, 7, 3, 16, 0)),
            ("FREQ=DAILY;INTERVAL=3", at(&london, 2018, 6, 1, 8, 0)),
            (
                "FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR",
                at(&ny, 2010, 1, 4, 7, 45),
            ),
            ("FREQ=HOURLY;INTERVAL=5", at(&RTz::UTC, 2024, 1, 1, 0, 15)),
            ("FREQ=HOURLY;INTERVAL=7", at(&ny, 2025, 2, 1, 0, 0)),
            ("FREQ=DAILY;COUNT=3000", at(&ny, 2020, 1, 1, 7, 0)),
            (
                "FREQ=WEEKLY;INTERVAL=2;COUNT=500",
                at(&london, 2015, 5, 5, 12, 0),
            ),
            // DST gap: 02:30 doesn't exist on 2027-03-14 in New York.
            ("FREQ=DAILY", at(&ny, 2020, 3, 1, 2, 30)),
        ];
        for (rule, start) in cases {
            let length = 1800;
            let got: Vec<i64> = expand_rrule(rule, start, length, now, until, far_deadline())
                .unwrap()
                .iter()
                .map(|d| d.timestamp())
                .collect();
            let want = reference(rule, start, now - ChronoDuration::seconds(length), until);
            assert!(!want.is_empty(), "{rule}");
            assert_eq!(got, want, "{rule} from {start}");
        }
        // A COUNT series that ended years ago is recognised without a walk.
        let start = at(&ny, 2020, 1, 1, 7, 0);
        assert!(matches!(
            fast_forward(
                &rule_parts("FREQ=DAILY;COUNT=10"),
                start,
                now - ChronoDuration::days(1)
            ),
            FastForward::Finished
        ));
        assert!(
            expand_rrule(
                "FREQ=DAILY;COUNT=10",
                start,
                1800,
                now,
                until,
                far_deadline()
            )
            .unwrap()
            .is_empty()
        );
        // BYSETPOS, and COUNT with BYxxx, are never moved.
        for rule in [
            "FREQ=WEEKLY;BYDAY=MO,TU;BYSETPOS=1",
            "FREQ=WEEKLY;BYDAY=MO;COUNT=900",
        ] {
            assert!(matches!(
                fast_forward(&rule_parts(rule), start, now),
                FastForward::Unchanged
            ));
        }
    }

    #[test]
    fn hostile_rules_finish_quickly() {
        let now = utc(2026, 11, 12, 10, 0);
        for (rule, expected) in [
            // Refused outright: only DTSTART (in 2000) is kept, so nothing.
            ("FREQ=MINUTELY", 0),
            ("FREQ=SECONDLY", 0),
            ("FREQ=MINUTELY;INTERVAL=1;BYSECOND=0", 0),
            // Fast-forwarded to the window.
            ("FREQ=HOURLY", 24),
            ("FREQ=HOURLY;BYMINUTE=0,30", 48),
            ("FREQ=DAILY;COUNT=1000000", 1),
        ] {
            let raw = cal(&format!(
                "BEGIN:VEVENT\r\nUID:h\r\nDTSTART:20000101T000000Z\r\nRRULE:{rule}\r\nEND:VEVENT\r\n"
            ));
            let t = Instant::now();
            let events = parse(&raw, now, 1);
            let took = t.elapsed();
            assert!(took < Duration::from_millis(100), "{rule} took {took:?}");
            assert_eq!(events.len(), expected, "{rule}");
        }
    }

    #[test]
    fn many_old_hourly_series_stay_cheap() {
        // QA's case: 300 HOURLY series since 2005 took 113 s before B-1.
        let body: String = (0..300)
            .map(|i| {
                format!(
                    "BEGIN:VEVENT\r\nUID:s{i}\r\nDTSTART:20050101T000000Z\r\nRRULE:FREQ=HOURLY\r\nEND:VEVENT\r\n"
                )
            })
            .collect();
        let t = Instant::now();
        let events = parse(&cal(&body), utc(2026, 11, 12, 10, 0), 1);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(events.len(), MAX_EVENTS_PER_FEED);
    }

    #[test]
    fn vevents_per_feed_are_capped() {
        let body: String = (0..MAX_VEVENTS_PER_FEED + 100)
            .map(|i| {
                format!("BEGIN:VEVENT\r\nUID:{i}\r\nDTSTART:20261112T120000Z\r\nEND:VEVENT\r\n")
            })
            .collect();
        assert_eq!(collect_events(&cal(&body)).len(), MAX_VEVENTS_PER_FEED);
    }

    /// Serves one HTTP response with a `len`-byte body on localhost.
    async fn serve_once(len: usize, send_length: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf).await;
            let head = if send_length {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: {len}\r\n\r\n"
                )
            } else {
                "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nConnection: close\r\n\r\n"
                    .to_string()
            };
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&vec![b'A'; len]).await;
            let _ = sock.shutdown().await;
        });
        format!("http://{addr}/feed.ics")
    }

    #[tokio::test]
    async fn feed_downloads_are_size_capped() {
        let cap = 4096;
        let small = fetch_feed_text(&serve_once(100, true).await, cap).await;
        assert_eq!(small.unwrap().len(), 100);
        // Refused from the header, and while streaming without one.
        let declared = fetch_feed_text(&serve_once(cap + 1, true).await, cap).await;
        assert!(declared.unwrap_err().contains("larger than"));
        let streamed = fetch_feed_text(&serve_once(cap * 4, false).await, cap).await;
        assert!(streamed.unwrap_err().contains("larger than"));
    }
}
