use futures::StreamExt;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PROXY_PORT: u16 = 18293;
const AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0";
const DEFAULT_REFERER: &str = "https://allmanga.to";

/// Local media server. Every request carries its own target and must present
/// `token`, so a web page cannot read files or relay URLs through it.
pub struct ProxyState {
    token: String,
    client: reqwest::Client,
}

impl ProxyState {
    pub fn new() -> Self {
        let bytes: [u8; 16] = rand::random();
        let token = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap();
        Self { token, client }
    }
}

pub async fn start_proxy(state: Arc<ProxyState>) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", PROXY_PORT)).await?;
    loop {
        if let Ok((stream, _)) = listener.accept().await {
            let state = state.clone();
            tokio::spawn(async move { handle(stream, state).await.ok(); });
        }
    }
}

fn parse_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params = query.split('&').filter_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        Some((k.to_string(), urlencoding::decode(v).ok()?.into_owned()))
    }).collect();
    (path.to_string(), params)
}

async fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {}\r\nConnection: close\r\n", status);
    for (k, v) in headers {
        head.push_str(&format!("{}: {}\r\n", k, v));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await
}

async fn handle(mut stream: TcpStream, state: Arc<ProxyState>) -> std::io::Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let mut parts = req.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" {
        return respond(&mut stream, "405 Method Not Allowed", &[("Content-Length", "0".into())]).await;
    }

    let (path, params) = parse_target(target);
    if params.get("k").map(String::as_str) != Some(state.token.as_str()) {
        return respond(&mut stream, "403 Forbidden", &[("Content-Length", "0".into())]).await;
    }

    let range_start = req.lines()
        .find(|l| l.to_lowercase().starts_with("range:"))
        .and_then(|l| l.split("bytes=").nth(1))
        .and_then(|r| r.split('-').next())
        .and_then(|s| s.trim().parse::<u64>().ok());

    match path.as_str() {
        "/video" => serve_remote(&mut stream, &state, &params, range_start).await,
        "/local" => serve_local(&mut stream, &params, range_start).await,
        _ => respond(&mut stream, "404 Not Found", &[("Content-Length", "0".into())]).await,
    }
}

async fn serve_local(stream: &mut TcpStream, params: &HashMap<String, String>, range_start: Option<u64>) -> std::io::Result<()> {
    let Some(p) = params.get("p") else {
        return respond(stream, "400 Bad Request", &[("Content-Length", "0".into())]).await;
    };
    let Ok(path) = crate::downloads::resolve_download_file(p) else {
        return respond(stream, "403 Forbidden", &[("Content-Length", "0".into())]).await;
    };

    let mut file = tokio::fs::File::open(&path).await?;
    let total = file.metadata().await?.len();
    let start = range_start.unwrap_or(0).min(total);
    let mut headers = vec![
        ("Content-Type", "video/mp4".to_string()),
        ("Content-Length", (total - start).to_string()),
        ("Accept-Ranges", "bytes".to_string()),
    ];
    let status = if range_start.is_some() {
        headers.push(("Content-Range", format!("bytes {}-{}/{}", start, total.saturating_sub(1), total)));
        "206 Partial Content"
    } else {
        "200 OK"
    };
    respond(stream, status, &headers).await?;

    if start > 0 { file.seek(SeekFrom::Start(start)).await?; }
    let mut buf = vec![0u8; 65536];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 || stream.write_all(&buf[..n]).await.is_err() { break; }
    }
    Ok(())
}

async fn serve_remote(stream: &mut TcpStream, state: &ProxyState, params: &HashMap<String, String>, range_start: Option<u64>) -> std::io::Result<()> {
    let Some(url) = params.get("u").filter(|u| u.starts_with("https://") || u.starts_with("http://")) else {
        return respond(stream, "400 Bad Request", &[("Content-Length", "0".into())]).await;
    };
    let referer = params.get("r").cloned().unwrap_or_else(|| DEFAULT_REFERER.to_string());

    let mut req = state.client.get(url).header("Referer", referer).header("User-Agent", AGENT);
    if let Some(start) = range_start {
        req = req.header("Range", format!("bytes={}-", start));
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => return respond(stream, "502 Bad Gateway", &[("Content-Length", "0".into())]).await,
    };

    let status = format!("{} {}", resp.status().as_u16(), resp.status().canonical_reason().unwrap_or(""));
    let mut headers: Vec<(&str, String)> = Vec::new();
    for name in ["Content-Type", "Content-Length", "Content-Range", "Accept-Ranges"] {
        if let Some(v) = resp.headers().get(name).and_then(|v| v.to_str().ok()) {
            headers.push((name, v.to_string()));
        }
    }
    respond(stream, &status, &headers).await?;

    let mut body = resp.bytes_stream();
    while let Some(Ok(chunk)) = body.next().await {
        if stream.write_all(&chunk).await.is_err() { break; }
    }
    Ok(())
}

/// URL the video element should play for a remote stream.
#[tauri::command]
pub fn proxy_url(url: String, referrer: Option<String>, state: State<'_, Arc<ProxyState>>) -> Result<String, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only http(s) URLs can be proxied".to_string());
    }
    let referrer = referrer.unwrap_or_else(|| DEFAULT_REFERER.to_string());
    Ok(format!(
        "http://127.0.0.1:{}/video?k={}&u={}&r={}",
        PROXY_PORT, state.token, urlencoding::encode(&url), urlencoding::encode(&referrer)
    ))
}

/// URL the video element should play for a downloaded file.
#[tauri::command]
pub fn local_url(path: String, state: State<'_, Arc<ProxyState>>) -> Result<String, String> {
    let file = crate::downloads::resolve_download_file(&path)?;
    Ok(format!(
        "http://127.0.0.1:{}/local?k={}&p={}",
        PROXY_PORT, state.token, urlencoding::encode(&file.to_string_lossy())
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_target;

    #[test]
    fn parses_path_and_decoded_params() {
        let (path, params) = parse_target("/video?k=abc&u=https%3A%2F%2Fx.com%2Fa%20b.mp4&r=");
        assert_eq!(path, "/video");
        assert_eq!(params.get("k").map(String::as_str), Some("abc"));
        assert_eq!(params.get("u").map(String::as_str), Some("https://x.com/a b.mp4"));
        assert_eq!(params.get("r").map(String::as_str), Some(""));
    }

    mod handler {
        use super::super::{handle, ProxyState};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        /// Fake upstream that answers every connection with a fixed body.
        async fn spawn_upstream() -> String {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                loop {
                    let Ok((mut s, _)) = listener.accept().await else { continue };
                    tokio::spawn(async move {
                        let mut buf = [0u8; 4096];
                        s.read(&mut buf).await.ok();
                        let body = b"hello-video";
                        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        s.write_all(head.as_bytes()).await.ok();
                        s.write_all(body).await.ok();
                    });
                }
            });
            format!("http://{}/clip.mp4", addr)
        }

        /// Sends one GET through a real proxy connection and returns the raw response.
        async fn get(state: Arc<ProxyState>, target: &str) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (s, _) = listener.accept().await.unwrap();
                handle(s, state).await.ok();
            });
            let mut client = TcpStream::connect(addr).await.unwrap();
            client.write_all(format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", target).as_bytes()).await.unwrap();
            let mut out = String::new();
            client.read_to_string(&mut out).await.unwrap();
            out
        }

        #[tokio::test]
        async fn rejects_missing_token() {
            let state = Arc::new(ProxyState::new());
            let upstream = spawn_upstream().await;
            let resp = get(state, &format!("/video?k=wrong&u={}", urlencoding::encode(&upstream))).await;
            assert!(resp.starts_with("HTTP/1.1 403"), "{}", resp);
        }

        #[tokio::test]
        async fn relays_remote_stream_with_token_and_no_cors_header() {
            let state = Arc::new(ProxyState::new());
            let upstream = spawn_upstream().await;
            let target = format!("/video?k={}&u={}", state.token, urlencoding::encode(&upstream));
            let resp = get(state, &target).await;
            assert!(resp.starts_with("HTTP/1.1 200"), "{}", resp);
            assert!(resp.ends_with("hello-video"), "{}", resp);
            assert!(!resp.to_ascii_lowercase().contains("access-control-allow-origin"), "{}", resp);
        }

        #[tokio::test]
        async fn refuses_local_files_outside_download_folder() {
            let state = Arc::new(ProxyState::new());
            let outside = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
            let target = format!("/local?k={}&p={}", state.token, urlencoding::encode(&outside));
            let resp = get(state, &target).await;
            assert!(resp.starts_with("HTTP/1.1 403"), "{}", resp);
        }

        #[tokio::test]
        async fn unknown_path_is_404() {
            let state = Arc::new(ProxyState::new());
            let resp = get(state.clone(), &format!("/nope?k={}", state.token)).await;
            assert!(resp.starts_with("HTTP/1.1 404"), "{}", resp);
        }
    }
}
