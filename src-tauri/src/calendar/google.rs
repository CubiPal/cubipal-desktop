use super::oauth::{generate_pkce, LoopbackServer};
use super::CalendarEvent;
use chrono::{DateTime, Utc};
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
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=https://www.googleapis.com/auth/calendar.readonly&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        client_id,
        url_encode(&redirect_uri),
        pkce.challenge
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

            let (start_epoch, is_all_day) = match item.start {
                Some(ref s) if s.date_time.is_some() => {
                    let dt = DateTime::parse_from_rfc3339(s.date_time.as_ref().unwrap())
                        .map(|d| d.timestamp())
                        .unwrap_or(0);
                    (dt, false)
                }
                Some(ref s) if s.date.is_some() => {
                    // Date-only (all day)
                    (0, true)
                }
                _ => (0, false),
            };

            let end_epoch = match item.end {
                Some(ref e) if e.date_time.is_some() => {
                    DateTime::parse_from_rfc3339(e.date_time.as_ref().unwrap())
                        .map(|d| d.timestamp())
                        .unwrap_or(start_epoch + 1800)
                }
                _ => start_epoch + 1800,
            };

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
