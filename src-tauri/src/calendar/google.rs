use super::CalendarEvent;
use super::oauth::{LoopbackServer, generate_pkce};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use std::time::Duration;

// Default client ID configured under KAN-38 (overridable via environment or settings)
pub const GOOGLE_DEFAULT_CLIENT_ID: &str =
    "772128439990-pgutm29ngflnbdplo9khmblprs7c0m83.apps.googleusercontent.com";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEventsResponse {
    items: Option<Vec<GoogleEventItem>>,
}

#[derive(Debug, Deserialize)]
struct GoogleEventItem {
    id: Option<String>,
    summary: Option<String>,
    start: Option<GoogleDateTime>,
    end: Option<GoogleDateTime>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleDateTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    date: Option<String>,
}

fn url_encode(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

pub async fn start_google_auth(client_id: &str) -> Result<(String, String), String> {
    let pkce = generate_pkce();
    let server = LoopbackServer::start()?;
    let redirect_uri = server.redirect_uri();

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=https://www.googleapis.com/auth/calendar.readonly&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        client_id,
        url_encode(&redirect_uri),
        pkce.challenge,
        server.state
    );

    // Open user's default browser
    open::that(&auth_url).map_err(|e| format!("Could not open browser: {}", e))?;

    // Wait for the redirect loopback
    let code = server.wait_for_code(120).await?;

    // Exchange code for tokens
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("code", &code),
        ("code_verifier", &pkce.verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", &redirect_uri),
    ];

    let res = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(format!("Google token exchange error: {}", err_text));
    }

    let token_resp: TokenResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    let refresh_token = token_resp
        .refresh_token
        .ok_or_else(|| "Google did not return a refresh token.".to_string())?;

    Ok((token_resp.access_token, refresh_token))
}

pub async fn refresh_google_access_token(
    client_id: &str,
    refresh_token: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let res = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Google refresh failed: {}", e))?;

    if !res.status().is_success() {
        return Err("Failed to refresh Google access token".to_string());
    }

    let token_resp: TokenResponse = res
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;

    Ok(token_resp.access_token)
}

pub async fn fetch_google_events(
    client_id: &str,
    refresh_token: &str,
) -> Result<Vec<CalendarEvent>, String> {
    let access_token = refresh_google_access_token(client_id, refresh_token).await?;

    let now = Utc::now();
    let horizon = now + chrono::Duration::hours(24);

    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime",
        url_encode(&now.to_rfc3339()),
        url_encode(&horizon.to_rfc3339())
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("Google Calendar fetch failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("Google API returned HTTP {}", res.status()));
    }

    let data: GoogleEventsResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse Google events: {}", e))?;

    let mut events = Vec::new();
    if let Some(items) = data.items {
        for item in items {
            if item.status.as_deref() == Some("cancelled") {
                continue;
            }

            let title = item
                .summary
                .unwrap_or_else(|| "(Untitled Meeting)".to_string());

            // An event whose start can't be read is skipped rather than
            // emitted at epoch 0 (which the "not ended" filter would drop anyway).
            let Some((start_epoch, is_all_day)) = item
                .start
                .as_ref()
                .and_then(|s| parse_google_time(s, &Local))
            else {
                continue;
            };

            let end_epoch = item
                .end
                .as_ref()
                .and_then(|e| parse_google_time(e, &Local))
                .map(|(t, _)| t)
                .filter(|&t| t > start_epoch)
                .unwrap_or(start_epoch + 1800);

            events.push(CalendarEvent {
                id: item.id.unwrap_or_default(),
                title,
                start: start_epoch,
                end: end_epoch,
                source: "google".to_string(),
                is_all_day,
                is_tentative: item.status.as_deref() == Some("tentative"),
            });
        }
    }

    Ok(events)
}

/// Converts a Google `start`/`end` object to epoch seconds. Timed events use
/// `dateTime` (RFC 3339); all-day events use `date`, which is midnight in the
/// user's local time zone `tz` (the `end` date is exclusive).
fn parse_google_time<Tz: TimeZone>(t: &GoogleDateTime, tz: &Tz) -> Option<(i64, bool)> {
    if let Some(dt) = t.date_time.as_deref() {
        return DateTime::parse_from_rfc3339(dt)
            .ok()
            .map(|d| (d.timestamp(), false));
    }
    let date = NaiveDate::parse_from_str(t.date.as_deref()?, "%Y-%m-%d").ok()?;
    // Midnight can be skipped by a DST change in a few zones; fall back to 01:00.
    [0, 1]
        .into_iter()
        .filter_map(|h| date.and_hms_opt(h, 0, 0))
        .find_map(|naive| tz.from_local_datetime(&naive).earliest())
        .map(|d| (d.timestamp(), true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn gdt(date_time: Option<&str>, date: Option<&str>) -> GoogleDateTime {
        GoogleDateTime {
            date_time: date_time.map(String::from),
            date: date.map(String::from),
        }
    }

    #[test]
    fn parses_timed_events() {
        let t = gdt(Some("2026-11-01T15:00:00Z"), None);
        assert_eq!(parse_google_time(&t, &Utc), Some((1_793_545_200, false)));
        let t = gdt(Some("2026-11-01T10:00:00-05:00"), None);
        assert_eq!(parse_google_time(&t, &Utc), Some((1_793_545_200, false)));
        assert_eq!(parse_google_time(&gdt(Some("garbage"), None), &Utc), None);
    }

    #[test]
    fn parses_all_day_events_at_local_midnight() {
        let t = gdt(None, Some("2026-11-01"));
        assert_eq!(parse_google_time(&t, &Utc), Some((1_793_491_200, true)));
        let pst = FixedOffset::west_opt(8 * 3600).unwrap();
        assert_eq!(
            parse_google_time(&t, &pst),
            Some((1_793_491_200 + 8 * 3600, true))
        );
    }

    #[test]
    fn missing_or_bad_times_give_none() {
        assert_eq!(parse_google_time(&gdt(None, None), &Utc), None);
        assert_eq!(
            parse_google_time(&gdt(None, Some("2026-13-01")), &Utc),
            None
        );
    }
}
