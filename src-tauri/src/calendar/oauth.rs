use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::distributions::Alphanumeric;
use rand::{Rng, thread_rng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::TcpListener;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Largest request head we read from the browser before giving up on it.
const MAX_REQUEST_BYTES: usize = 8 * 1024;
/// How long one browser connection may take to send its request line.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);

const SUCCESS_HTML: &str = "<!DOCTYPE html><html><body style='font-family:-apple-system,BlinkMacSystemFont,Segoe UI,Roboto,sans-serif;text-align:center;padding:60px 20px;background:#f8fafc;'><div style='max-width:400px;margin:auto;background:white;padding:30px;border-radius:12px;border:1px solid #e2e8f0;box-shadow:0 2px 4px rgba(0,0,0,0.05);'><h2 style='color:#16a34a;margin-bottom:10px;'>✓ Account Connected!</h2><p style='color:#64748b;font-size:14px;line-height:1.5;'>Your calendar is now connected to CubiPal. You can close this tab and return to the app.</p></div></body></html>";

const FAILURE_HTML: &str = "<!DOCTYPE html><html><body style='font-family:-apple-system,BlinkMacSystemFont,Segoe UI,Roboto,sans-serif;text-align:center;padding:60px 20px;background:#f8fafc;'><div style='max-width:400px;margin:auto;background:white;padding:30px;border-radius:12px;border:1px solid #e2e8f0;box-shadow:0 2px 4px rgba(0,0,0,0.05);'><h2 style='color:#dc2626;margin-bottom:10px;'>Calendar not connected</h2><p style='color:#64748b;font-size:14px;line-height:1.5;'>Sign-in was not completed. You can close this tab and try again from the CubiPal app.</p></div></body></html>";

pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

fn random_alphanumeric(len: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

pub fn generate_pkce() -> PkceChallenge {
    let verifier = random_alphanumeric(64);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    PkceChallenge {
        verifier,
        challenge,
    }
}

pub struct LoopbackServer {
    pub listener: TcpListener,
    pub port: u16,
    /// Random value sent as the OAuth `state` parameter; the callback must echo it.
    pub state: String,
}

/// What one request to the loopback server means for the sign-in flow.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Callback {
    /// The provider redirected back with an authorization code.
    Code(String),
    /// The provider redirected back with `error=` (e.g. the user cancelled).
    Error(String),
    /// A callback whose `state` doesn't match this sign-in; ignored.
    BadState,
    /// Anything else (favicon, pre-connect, unrelated path); ignored.
    Ignore,
}

impl LoopbackServer {
    pub fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        // Required by tokio::net::TcpListener::from_std.
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        Ok(Self {
            listener,
            port,
            state: random_alphanumeric(32),
        })
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/oauth/callback", self.port)
    }

    /// Serves the loopback redirect until a callback with a matching `state`
    /// arrives (or the timeout passes). Each connection is handled on its own
    /// task, so an idle browser pre-connect or a favicon request never blocks
    /// or consumes the flow.
    pub async fn wait_for_code(self, timeout_secs: u64) -> Result<String, String> {
        let listener =
            tokio::net::TcpListener::from_std(self.listener).map_err(|e| e.to_string())?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<String, String>>(4);

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    return Err("Authentication timed out waiting for browser response.".to_string());
                }
                Some(result) = rx.recv() => return result,
                accepted = listener.accept() => {
                    let (stream, _) = accepted.map_err(|e| format!("Socket accept error: {}", e))?;
                    let tx = tx.clone();
                    let state = self.state.clone();
                    tokio::spawn(async move {
                        if let Some(result) = handle_connection(stream, &state).await {
                            let _ = tx.send(result).await;
                        }
                    });
                }
            }
        }
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    expected_state: &str,
) -> Option<Result<String, String>> {
    let request = tokio::time::timeout(REQUEST_READ_TIMEOUT, read_request_head(&mut stream))
        .await
        .ok()?
        .ok()?;

    let (status, body, result) = match parse_callback_request(&request, expected_state) {
        Callback::Code(code) => ("200 OK", SUCCESS_HTML, Some(Ok(code))),
        Callback::Error(msg) => ("200 OK", FAILURE_HTML, Some(Err(msg))),
        Callback::BadState => ("400 Bad Request", "Invalid sign-in request.", None),
        Callback::Ignore => ("404 Not Found", "Not found", None),
    };
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    result
}

/// Reads until the end of the HTTP request head (or 8 KiB / EOF).
async fn read_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= MAX_REQUEST_BYTES {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Interprets the request line of a browser request to the loopback server.
/// Query values are percent-decoded (Google codes start with `4/`, often sent
/// as `4%2F`).
pub(crate) fn parse_callback_request(request: &str, expected_state: &str) -> Callback {
    let Some(target) = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
    else {
        return Callback::Ignore;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/oauth/callback" {
        return Callback::Ignore;
    }

    let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    let state_ok = params.get("state").is_some_and(|s| s == expected_state);

    if let Some(error) = params.get("error") {
        if !state_ok {
            return Callback::BadState;
        }
        return Callback::Error(if error == "access_denied" {
            "Sign-in was cancelled.".to_string()
        } else {
            let detail = params
                .get("error_description")
                .filter(|d| !d.is_empty())
                .unwrap_or(error);
            format!("Sign-in failed: {}", detail)
        });
    }

    match params.get("code").filter(|c| !c.is_empty()) {
        Some(code) if state_ok => Callback::Code(code.clone()),
        Some(_) => Callback::BadState,
        None => Callback::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = "abc123";

    fn req(target: &str) -> String {
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
    }

    #[test]
    fn extracts_code_with_matching_state() {
        assert_eq!(
            parse_callback_request(&req("/oauth/callback?code=abc&state=abc123"), STATE),
            Callback::Code("abc".into())
        );
        assert_eq!(
            parse_callback_request(&req("/oauth/callback?state=abc123&scope=x&code=abc"), STATE),
            Callback::Code("abc".into())
        );
    }

    #[test]
    fn code_is_percent_decoded() {
        assert_eq!(
            parse_callback_request(
                &req("/oauth/callback?code=4%2F0AbC&scope=x&state=abc123"),
                STATE
            ),
            Callback::Code("4/0AbC".into())
        );
    }

    #[test]
    fn state_must_match() {
        assert_eq!(
            parse_callback_request(&req("/oauth/callback?code=abc&state=nope"), STATE),
            Callback::BadState
        );
        assert_eq!(
            parse_callback_request(&req("/oauth/callback?code=abc"), STATE),
            Callback::BadState
        );
        assert_eq!(
            parse_callback_request(&req("/oauth/callback?error=access_denied"), STATE),
            Callback::BadState
        );
    }

    #[test]
    fn error_is_reported_immediately() {
        assert_eq!(
            parse_callback_request(
                &req("/oauth/callback?error=access_denied&state=abc123"),
                STATE
            ),
            Callback::Error("Sign-in was cancelled.".into())
        );
        assert_eq!(
            parse_callback_request(
                &req(
                    "/oauth/callback?error=invalid_scope&error_description=Bad+scope&state=abc123"
                ),
                STATE
            ),
            Callback::Error("Sign-in failed: Bad scope".into())
        );
    }

    #[test]
    fn stray_requests_are_ignored() {
        assert_eq!(
            parse_callback_request(&req("/favicon.ico"), STATE),
            Callback::Ignore
        );
        assert_eq!(
            parse_callback_request(&req("/oauth/callback"), STATE),
            Callback::Ignore
        );
        assert_eq!(
            parse_callback_request(&req("/oauth/callback?code=&state=abc123"), STATE),
            Callback::Ignore
        );
        assert_eq!(parse_callback_request("", STATE), Callback::Ignore);
        assert_eq!(
            parse_callback_request(&req("/other?code=abc&state=abc123"), STATE),
            Callback::Ignore
        );
    }

    #[test]
    fn pkce_challenge_matches_verifier() {
        let a = generate_pkce();
        let b = generate_pkce();
        assert_eq!(a.verifier.len(), 64);
        assert!(a.verifier.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(
            a.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(a.verifier.as_bytes()))
        );
        assert!(!a.challenge.contains('='));
        assert_ne!(a.verifier, b.verifier);
    }

    /// Reproduces QA-04: an idle pre-connect, a favicon request and a delayed
    /// callback request must not drop or block the flow.
    #[tokio::test]
    async fn loopback_survives_idle_and_stray_connections() {
        use tokio::net::TcpStream;

        let server = LoopbackServer::start().unwrap();
        let port = server.port;
        let state = server.state.clone();
        let waiter = tokio::spawn(server.wait_for_code(10));

        // Idle pre-connect that never sends anything.
        let _idle = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

        // Stray favicon request gets a 404 and doesn't end the flow.
        let mut fav = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        fav.write_all(req("/favicon.ico").as_bytes()).await.unwrap();
        let mut resp = String::new();
        fav.read_to_string(&mut resp).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 404"), "{resp}");

        // Real callback, sent 50 ms after connecting.
        let mut cb = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        cb.write_all(req(&format!("/oauth/callback?code=4%2Fxyz&state={state}")).as_bytes())
            .await
            .unwrap();
        let mut resp = String::new();
        cb.read_to_string(&mut resp).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");

        assert_eq!(waiter.await.unwrap(), Ok("4/xyz".to_string()));
    }

    #[tokio::test]
    async fn loopback_returns_error_on_denial() {
        use tokio::net::TcpStream;

        let server = LoopbackServer::start().unwrap();
        let port = server.port;
        let state = server.state.clone();
        let waiter = tokio::spawn(server.wait_for_code(10));

        let mut cb = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        cb.write_all(
            req(&format!(
                "/oauth/callback?error=access_denied&state={state}"
            ))
            .as_bytes(),
        )
        .await
        .unwrap();
        assert_eq!(
            waiter.await.unwrap(),
            Err("Sign-in was cancelled.".to_string())
        );
    }
}
