use std::sync::Arc;
use tokio::sync::Mutex;

const PROXY_PORT: u16 = 18293;

pub struct ProxyState {
    pub current_url: Arc<Mutex<String>>,
    pub referrer: Arc<Mutex<String>>,
}

impl ProxyState {
    pub fn new() -> Self {
        Self {
            current_url: Arc::new(Mutex::new(String::new())),
            referrer: Arc::new(Mutex::new("https://allmanga.to".to_string())),
        }
    }
}

pub fn start_proxy(state: Arc<ProxyState>) {
    tokio::spawn(async move {
        use tokio::net::TcpListener;
        use tokio::io::{AsyncWriteExt, AsyncReadExt};

        let listener = TcpListener::bind(format!("127.0.0.1:{}", PROXY_PORT)).await.unwrap();
        loop {
            if let Ok((mut stream, _)) = listener.accept().await {
                let state = state.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();

                    let url = state.current_url.lock().await.clone();
                    let refr = state.referrer.lock().await.clone();

                    if url.is_empty() {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
                        return;
                    }

                    // Parse Range header
                    let range_start = req.lines()
                        .find(|l| l.to_lowercase().starts_with("range:"))
                        .and_then(|l| l.split("bytes=").nth(1))
                        .and_then(|r| r.split('-').next())
                        .and_then(|s| s.trim().parse::<u64>().ok());

                    let client = reqwest::Client::builder().build().unwrap();
                    let mut req_builder = client.get(&url)
                        .header("Referer", &refr)
                        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0");

                    if let Some(start) = range_start {
                        req_builder = req_builder.header("Range", format!("bytes={}-", start));
                    }

                    match req_builder.send().await {
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            let ct = resp.headers().get("content-type")
                                .and_then(|v| v.to_str().ok()).unwrap_or("video/mp4").to_string();
                            let cl = resp.headers().get("content-length")
                                .and_then(|v| v.to_str().ok()).unwrap_or("0").to_string();
                            let cr = resp.headers().get("content-range")
                                .and_then(|v| v.to_str().ok()).map(|s| s.to_string());
                            let accept_ranges = resp.headers().get("accept-ranges")
                                .and_then(|v| v.to_str().ok()).unwrap_or("bytes").to_string();

                            let status_line = if status == 206 || range_start.is_some() { "206 Partial Content" } else { "200 OK" };
                            let mut header = format!("HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n", status_line, ct, cl, accept_ranges);
                            if let Some(cr) = cr { header.push_str(&format!("Content-Range: {}\r\n", cr)); }
                            header.push_str("\r\n");

                            let _ = stream.write_all(header.as_bytes()).await;

                            use futures::StreamExt;
                            let mut body = resp.bytes_stream();
                            while let Some(Ok(chunk)) = body.next().await {
                                if stream.write_all(&chunk).await.is_err() { break; }
                            }
                        }
                        Err(_) => {
                            let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
                        }
                    }
                });
            }
        }
    });
}

#[tauri::command]
pub async fn set_proxy_url(url: String, referrer: Option<String>, state: tauri::State<'_, Arc<ProxyState>>) -> Result<String, String> {
    *state.current_url.lock().await = url;
    if let Some(r) = referrer { *state.referrer.lock().await = r; }
    Ok(format!("http://127.0.0.1:{}/video", PROXY_PORT))
}
