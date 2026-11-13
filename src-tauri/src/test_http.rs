//! A tiny scripted HTTP server on localhost for tests.

use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serves one connection per `(status, body)` in order, then stops. Returns
/// the `host:port` to call and the request lines received (`POST /api/x`).
pub async fn serve(
    replies: Vec<(&'static str, &'static str)>,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        for (status, body) in replies {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let line = request.lines().next().unwrap_or_default();
            let line = line.rsplit_once(' ').map_or(line, |(l, _)| l).to_string();
            log.lock().unwrap().push(line);
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (addr, seen)
}
