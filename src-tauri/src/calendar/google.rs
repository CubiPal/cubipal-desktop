use super::oauth::{LoopbackServer, generate_pkce};
use super::{CalendarEvent, ProviderError, is_revoked_grant};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use std::time::Duration;

// Default client ID configured under KAN-38. A build can override it with the
// GOOGLE_CLIENT_ID environment variable (the release workflow exports it).
const GOOGLE_DEFAULT_CLIENT_ID: &str =
    "772128439990-pgutm29ngflnbdplo9khmblprs7c0m83.apps.googleusercontent.com";

/// The OAuth client used for Google sign-in and token refresh.
pub struct OAuthClient {
    pub id: String,
    /// Google "Desktop app" OAuth clients must send their client secret to
    /// the token endpoint, even with PKCE. For installed apps Google treats
    /// this value as non-confidential (it ships inside every copy of the
    /// app), so embedding it at build time is the documented practice; PKCE
    /// and the loopback redirect are what protect the flow.
    pub secret: Option<String>,
}

impl OAuthClient {
    /// The client this build was made with: GOOGLE_CLIENT_ID and
    /// GOOGLE_CLIENT_SECRET from the build environment, when set.
    pub fn built_in() -> Self {
        let env_id = option_env!("GOOGLE_CLIENT_ID")
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let secret = option_env!("GOOGLE_CLIENT_SECRET")
            .map(str::trim)
            .filter(|s| !s.is_empty());
        Self::from_build(env_id, secret)
    }

    fn from_build(env_id: Option<&str>, secret: Option<&str>) -> Self {
        OAuthClient {
            id: env_id.unwrap_or(GOOGLE_DEFAULT_CLIENT_ID).to_string(),
            secret: secret.map(String::from),
        }
    }

    /// A client ID typed in by the user; the built-in secret doesn't belong
    /// to it, so none is sent.
    pub fn custom(id: String) -> Self {
        OAuthClient {
            id: id.trim().to_string(),
            secret: None,
        }
    }

    /// Token-endpoint form fields, with the secret when there is one.
    fn token_params<'a>(&'a self, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        let mut params = vec![("client_id", self.id.as_str())];
        if let Some(secret) = self.secret.as_deref() {
            params.push(("client_secret", secret));
        }
        params.extend_from_slice(extra);
        params
    }
}

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

pub async fn start_google_auth(oauth: &OAuthClient) -> Result<(String, String), String> {
    let pkce = generate_pkce();
    let server = LoopbackServer::start()?;
    let redirect_uri = server.redirect_uri();

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope=https://www.googleapis.com/auth/calendar.readonly&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
        url_encode(&oauth.id),
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
    let params = oauth.token_params(&[
        ("code", &code),
        ("code_verifier", &pkce.verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", &redirect_uri),
    ]);

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
    oauth: &OAuthClient,
    refresh_token: &str,
) -> Result<String, ProviderError> {
    let client = reqwest::Client::new();
    let params = oauth.token_params(&[
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]);

    let res = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Google refresh failed: {}", e))?;

    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        if is_revoked_grant(status.as_u16(), &body) {
            return Err(ProviderError::Revoked);
        }
        return Err(format!("Failed to refresh Google access token (HTTP {})", status).into());
    }

    let token_resp: TokenResponse = res
        .json()
        .await
        .map_err(|e| format!("Parse error: {}", e))?;

    Ok(token_resp.access_token)
}

pub async fn fetch_google_events(
    oauth: &OAuthClient,
    refresh_token: &str,
) -> Result<Vec<CalendarEvent>, ProviderError> {
    let access_token = refresh_google_access_token(oauth, refresh_token).await?;

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
        return Err(format!("Google API returned HTTP {}", res.status()).into());
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
    fn client_secret_is_sent_only_with_the_built_in_client() {
        let built = OAuthClient::from_build(None, Some("s3cret"));
        assert_eq!(built.id, GOOGLE_DEFAULT_CLIENT_ID);
        let params = built.token_params(&[("grant_type", "refresh_token")]);
        assert_eq!(
            params,
            [
                ("client_id", GOOGLE_DEFAULT_CLIENT_ID),
                ("client_secret", "s3cret"),
                ("grant_type", "refresh_token")
            ]
        );
        let env = OAuthClient::from_build(Some("env-id"), None);
        assert_eq!(env.id, "env-id");
        assert!(
            !env.token_params(&[])
                .iter()
                .any(|(k, _)| *k == "client_secret")
        );
        let custom = OAuthClient::custom(" mine ".into());
        assert_eq!(custom.id, "mine");
        assert!(custom.secret.is_none());
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
