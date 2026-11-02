use super::CalendarEvent;
use super::oauth::{LoopbackServer, generate_pkce};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::time::Duration;

// Default client ID configured under KAN-39 (overridable via settings)
pub const MICROSOFT_DEFAULT_CLIENT_ID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Debug, Deserialize)]
struct MsTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MsCalendarViewResponse {
    value: Option<Vec<MsEventItem>>,
}

#[derive(Debug, Deserialize)]
struct MsEventItem {
    id: Option<String>,
    subject: Option<String>,
    start: Option<MsDateTimeZone>,
    end: Option<MsDateTimeZone>,
    #[serde(rename = "isAllDay")]
    is_all_day: Option<bool>,
    #[serde(rename = "responseStatus")]
    response_status: Option<MsResponseStatus>,
}

#[derive(Debug, Deserialize)]
struct MsDateTimeZone {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
    #[serde(rename = "timeZone")]
    #[allow(dead_code)]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MsResponseStatus {
    response: Option<String>,
}

fn url_encode(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

pub async fn start_microsoft_auth(client_id: &str) -> Result<(String, String), String> {
    let pkce = generate_pkce();
    let server = LoopbackServer::start()?;
    let redirect_uri = server.redirect_uri();

    let auth_url = format!(
        "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id={}&redirect_uri={}&response_type=code&scope=Calendars.Read%20offline_access&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        client_id,
        url_encode(&redirect_uri),
        pkce.challenge
    );

    open::that(&auth_url).map_err(|e| format!("Could not open browser: {}", e))?;

    let code = server.wait_for_code(120).await?;

    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("code", &code),
        ("code_verifier", &pkce.verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", &redirect_uri),
    ];

    let res = client
        .post("https://login.microsoftonline.com/common/oauth2/v2.0/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(format!("Microsoft token exchange error: {}", err_text));
    }

    let token_resp: MsTokenResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse Microsoft token response: {}", e))?;

    let refresh_token = token_resp
        .refresh_token
        .ok_or_else(|| "Microsoft did not return a refresh token.".to_string())?;

    Ok((token_resp.access_token, refresh_token))
}

pub async fn refresh_microsoft_access_token(
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
        .post("https://login.microsoftonline.com/common/oauth2/v2.0/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Microsoft refresh request failed: {}", e))?;

    if !res.status().is_success() {
        return Err("Failed to refresh Microsoft access token".to_string());
    }

    let token_resp: MsTokenResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    Ok(token_resp.access_token)
}

pub async fn fetch_microsoft_events(
    client_id: &str,
    refresh_token: &str,
) -> Result<Vec<CalendarEvent>, String> {
    let access_token = refresh_microsoft_access_token(client_id, refresh_token).await?;

    let now = Utc::now();
    let horizon = now + chrono::Duration::hours(24);

    let url = format!(
        "https://graph.microsoft.com/v1.0/me/calendarView?startDateTime={}&endDateTime={}&$orderby=start/dateTime",
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
        .header("Prefer", "outlook.timezone=\"UTC\"")
        .send()
        .await
        .map_err(|e| format!("Microsoft Calendar fetch failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("Microsoft Graph returned HTTP {}", res.status()));
    }

    let data: MsCalendarViewResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse Microsoft events: {}", e))?;

    let mut events = Vec::new();
    if let Some(items) = data.value {
        for item in items {
            let resp = item
                .response_status
                .as_ref()
                .and_then(|r| r.response.as_deref())
                .unwrap_or("none");

            if resp == "declined" {
                continue;
            }

            let title = item
                .subject
                .unwrap_or_else(|| "(Untitled Meeting)".to_string());

            let is_all_day = item.is_all_day.unwrap_or(false);

            let start_epoch = item
                .start
                .as_ref()
                .and_then(|s| s.date_time.as_ref())
                .and_then(|dt| {
                    DateTime::parse_from_rfc3339(dt).ok().or_else(|| {
                        // Try parsing without timezone if sent as naive UTC
                        chrono::NaiveDateTime::parse_from_str(dt, "%Y-%m-%dT%H:%M:%S")
                            .ok()
                            .map(|n| n.and_utc().fixed_offset())
                    })
                })
                .map(|d| d.timestamp())
                .unwrap_or(0);

            let end_epoch = item
                .end
                .as_ref()
                .and_then(|e| e.date_time.as_ref())
                .and_then(|dt| {
                    DateTime::parse_from_rfc3339(dt).ok().or_else(|| {
                        chrono::NaiveDateTime::parse_from_str(dt, "%Y-%m-%dT%H:%M:%S")
                            .ok()
                            .map(|n| n.and_utc().fixed_offset())
                    })
                })
                .map(|d| d.timestamp())
                .unwrap_or(start_epoch + 1800);

            events.push(CalendarEvent {
                id: item.id.unwrap_or_default(),
                title,
                start: start_epoch,
                end: end_epoch,
                source: "microsoft".to_string(),
                is_all_day,
                is_tentative: resp == "tentativelyAccepted" || resp == "notResponded",
            });
        }
    }

    Ok(events)
}
