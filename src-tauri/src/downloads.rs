use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use tokio::sync::mpsc;

use crate::store;

const DOWNLOADS_FILE: &str = "downloads.json";
const AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0";

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum DownloadStatus { Queued, Downloading, Done, Failed }

#[derive(Serialize, Deserialize, Clone)]
pub struct DownloadItem {
    pub id: String,
    pub anime_id: String,
    pub anime_name: String,
    pub episode: String,
    pub mode: String,
    pub status: DownloadStatus,
    pub progress: f64,
    pub file_path: String,
    pub error: Option<String>,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
}

pub struct DownloadManager {
    pub items: Arc<Mutex<Vec<DownloadItem>>>,
    pub cancel_tx: Arc<Mutex<HashMap<String, mpsc::Sender<()>>>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        let mut items: Vec<DownloadItem> = store::load(DOWNLOADS_FILE);
        // Anything still active was cut off when the app last closed.
        for item in items.iter_mut().filter(|i| matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading)) {
            item.status = DownloadStatus::Failed;
            item.error = Some("Interrupted when the app closed".to_string());
        }
        store::save(DOWNLOADS_FILE, &items);
        Self { items: Arc::new(Mutex::new(items)), cancel_tx: Arc::new(Mutex::new(HashMap::new())) }
    }
}

fn download_dir() -> PathBuf {
    let dir = dirs::video_dir().unwrap_or_else(|| dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("ani-des");
    fs::create_dir_all(&dir).ok();
    dir
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn check_within(base: &Path, file: &Path) -> Result<PathBuf, String> {
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    let file = file.canonicalize().map_err(|e| e.to_string())?;
    if file.starts_with(&base) && file.is_file() {
        Ok(file)
    } else {
        Err("File is outside the download folder".to_string())
    }
}

/// Canonical path of a downloaded file, or an error if it is not inside the download folder.
pub fn resolve_download_file(path: &str) -> Result<PathBuf, String> {
    check_within(&download_dir(), Path::new(path))
}

#[tauri::command]
pub async fn start_download(anime_id: String, anime_name: String, episode: String, mode: String, dm: State<'_, DownloadManager>) -> Result<String, String> {
    let id = format!("{}_{}", anime_id, episode);
    {
        let items = dm.items.lock().unwrap();
        if items.iter().any(|i| i.id == id && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Done)) {
            return Err("Already downloaded or in progress".to_string());
        }
    }

    let item = DownloadItem {
        id: id.clone(), anime_id: anime_id.clone(), anime_name: anime_name.clone(),
        episode: episode.clone(), mode: mode.clone(),
        status: DownloadStatus::Queued, progress: 0.0, file_path: String::new(), error: None,
        downloaded_bytes: 0, total_bytes: 0,
    };

    { let mut items = dm.items.lock().unwrap(); items.retain(|i| i.id != id); items.push(item); }

    let items_arc = dm.items.clone();
    let cancel_arc = dm.cancel_tx.clone();
    let (tx, mut rx) = mpsc::channel::<()>(1);
    cancel_arc.lock().unwrap().insert(id.clone(), tx);

    let task_id = id.clone();
    tokio::spawn(async move {
        do_download(id, anime_id, anime_name, episode, mode, items_arc, &mut rx).await;
        cancel_arc.lock().unwrap().remove(&task_id);
    });

    Ok("queued".to_string())
}

fn with_item(items: &Arc<Mutex<Vec<DownloadItem>>>, id: &str, f: impl FnOnce(&mut DownloadItem)) {
    if let Ok(mut list) = items.lock() {
        if let Some(item) = list.iter_mut().find(|i| i.id == id) { f(item); }
    }
}

/// Changes an item and persists the list. Use for status changes, not per-chunk progress.
fn update_item(items: &Arc<Mutex<Vec<DownloadItem>>>, id: &str, f: impl FnOnce(&mut DownloadItem)) {
    if let Ok(mut list) = items.lock() {
        if let Some(item) = list.iter_mut().find(|i| i.id == id) { f(item); }
        store::save(DOWNLOADS_FILE, &*list);
    }
}

fn fail(items: &Arc<Mutex<Vec<DownloadItem>>>, id: &str, msg: String) {
    update_item(items, id, |i| { i.status = DownloadStatus::Failed; i.error = Some(msg); });
}

async fn do_download(id: String, anime_id: String, anime_name: String, episode: String, mode: String, items: Arc<Mutex<Vec<DownloadItem>>>, cancel: &mut mpsc::Receiver<()>) {
    update_item(&items, &id, |i| { i.status = DownloadStatus::Downloading; i.progress = 5.0; });

    let links = match get_links_for_download(&anime_id, &episode, &mode).await {
        Ok(l) => l,
        Err(e) => return fail(&items, &id, e),
    };
    let Some(url) = links.first().cloned() else {
        return fail(&items, &id, "No video sources found".to_string());
    };
    if url.contains(".m3u8") {
        return download_hls(&id, &anime_name, &episode, &url, &items, cancel).await;
    }

    update_item(&items, &id, |i| i.progress = 15.0);

    let dir = download_dir().join(sanitize(&anime_name));
    fs::create_dir_all(&dir).ok();
    let file_path = dir.join(format!("Episode_{}.mp4", episode));

    // Connect timeout only: a whole-download timeout would kill long files on slow links.
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap();
    let resp = match client.get(&url)
        .header("Referer", "https://allmanga.to")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0")
        .send().await {
        Ok(r) => r,
        Err(e) => return fail(&items, &id, e.to_string()),
    };

    let total = resp.content_length().unwrap_or(0);
    update_item(&items, &id, |i| i.total_bytes = total);
    let mut file = match fs::File::create(&file_path) {
        Ok(f) => f,
        Err(e) => return fail(&items, &id, e.to_string()),
    };

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    loop {
        let chunk = tokio::select! {
            Some(_) = cancel.recv() => {
                fs::remove_file(&file_path).ok();
                return fail(&items, &id, "Cancelled".to_string());
            }
            c = stream.next() => c,
        };
        let Some(chunk) = chunk else { break };
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                fs::remove_file(&file_path).ok();
                return fail(&items, &id, e.to_string());
            }
        };
        if file.write_all(&bytes).is_err() {
            fs::remove_file(&file_path).ok();
            return fail(&items, &id, "Write error".to_string());
        }
        downloaded += bytes.len() as u64;
        let pct = if total > 0 { 15.0 + (downloaded as f64 / total as f64) * 85.0 } else { 50.0 };
        with_item(&items, &id, |i| { i.progress = pct; i.downloaded_bytes = downloaded; });
    }

    update_item(&items, &id, |i| {
        i.status = DownloadStatus::Done;
        i.progress = 100.0;
        i.downloaded_bytes = downloaded;
        i.file_path = file_path.to_string_lossy().to_string();
    });
}

/// Direct (non-HLS) file URLs for an episode, in provider order.
async fn get_links_for_download(show_id: &str, episode: &str, mode: &str) -> Result<Vec<String>, String> {
    let fetch = crate::api::fetch_source_list(show_id, episode, mode).await?;
    let sources = crate::api::parse_sources(&fetch.api_resp);
    let links = crate::api::resolve_sources(&sources).await;
    let mut urls: Vec<String> = links.into_iter().map(|l| l.url).collect();
    // Direct files first; HLS playlists are used only when no direct file exists.
    urls.sort_by_key(|u| u.contains(".m3u8"));
    Ok(urls)
}

/// Chooses the highest-bandwidth variant of a master playlist; None for a media playlist.
pub fn pick_variant(base: &reqwest::Url, text: &str) -> Option<reqwest::Url> {
    if !text.contains("#EXT-X-STREAM-INF") { return None; }
    let mut best: Option<(u64, String)> = None;
    let mut pending: Option<u64> = None;
    for line in text.lines().map(str::trim) {
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = attrs.split(',')
                .find_map(|kv| kv.trim().strip_prefix("BANDWIDTH="))
                .and_then(|v| v.parse().ok())
                .or(Some(0));
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(bw) = pending.take() {
                if best.as_ref().map_or(true, |(b, _)| bw > *b) { best = Some((bw, line.to_string())); }
            }
        }
    }
    best.and_then(|(_, uri)| base.join(&uri).ok())
}

/// Segment (duration seconds, absolute URL) pairs of a media playlist.
pub fn parse_media_playlist(base: &reqwest::Url, text: &str) -> Result<Vec<(f64, String)>, String> {
    if text.lines().any(|l| l.starts_with("#EXT-X-KEY:") && !l.contains("METHOD=NONE")) {
        return Err("Encrypted HLS streams cannot be downloaded yet".to_string());
    }
    if text.contains("#EXT-X-MAP:") {
        return Err("Fragmented HLS streams cannot be downloaded yet".to_string());
    }
    let mut segments = Vec::new();
    let mut duration = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            duration = rest.split(',').next().and_then(|d| d.trim().parse::<f64>().ok());
        } else if !line.is_empty() && !line.starts_with('#') {
            let uri = base.join(line).map_err(|e| e.to_string())?;
            segments.push((duration.take().unwrap_or(0.0), uri.to_string()));
        }
    }
    Ok(segments)
}

/// Playlist written beside a downloaded transport stream; the proxy serves it to the player.
fn sidecar_playlist(ts_name: &str, total_secs: f64) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{}\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:{:.3},\n{}\n#EXT-X-ENDLIST\n",
        total_secs.ceil().max(1.0) as u64, total_secs, ts_name
    )
}

fn site_headers() -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::REFERER, reqwest::header::HeaderValue::from_static("https://allmanga.to"));
    h.insert(reqwest::header::USER_AGENT, reqwest::header::HeaderValue::from_static(AGENT));
    h
}

async fn fetch_text(client: &reqwest::Client, url: &reqwest::Url) -> Result<String, String> {
    let resp = client.get(url.clone()).headers(site_headers()).send().await.map_err(|e| e.to_string())?;
    resp.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client.get(url).headers(site_headers()).send().await.map_err(|e| e.to_string())?;
    let bytes = resp.error_for_status().map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

/// Downloads every segment of an HLS stream into one `.ts` file plus a playlist beside it.
async fn download_hls(id: &str, anime_name: &str, episode: &str, url: &str, items: &Arc<Mutex<Vec<DownloadItem>>>, cancel: &mut mpsc::Receiver<()>) {
    let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap();
    let mut base = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(e) => return fail(items, id, e.to_string()),
    };
    let mut text = match fetch_text(&client, &base).await {
        Ok(t) => t,
        Err(e) => return fail(items, id, e),
    };
    if let Some(variant) = pick_variant(&base, &text) {
        text = match fetch_text(&client, &variant).await {
            Ok(t) => t,
            Err(e) => return fail(items, id, e),
        };
        base = variant;
    }
    let segments = match parse_media_playlist(&base, &text) {
        Ok(s) => s,
        Err(e) => return fail(items, id, e),
    };
    if segments.is_empty() {
        return fail(items, id, "Stream playlist has no segments".to_string());
    }
    let total_secs: f64 = segments.iter().map(|(d, _)| d).sum();

    let dir = download_dir().join(sanitize(anime_name));
    fs::create_dir_all(&dir).ok();
    let ts_name = format!("Episode_{}.ts", episode);
    let ts_path = dir.join(&ts_name);
    let playlist_path = ts_path.with_extension("m3u8");
    let mut file = match fs::File::create(&ts_path) {
        Ok(f) => f,
        Err(e) => return fail(items, id, e.to_string()),
    };

    use std::io::Write;
    let count = segments.len();
    let mut downloaded: u64 = 0;
    for (i, (_, uri)) in segments.iter().enumerate() {
        let bytes = tokio::select! {
            Some(_) = cancel.recv() => {
                fs::remove_file(&ts_path).ok();
                return fail(items, id, "Cancelled".to_string());
            }
            r = fetch_bytes(&client, uri) => r,
        };
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                fs::remove_file(&ts_path).ok();
                return fail(items, id, e);
            }
        };
        if file.write_all(&bytes).is_err() {
            fs::remove_file(&ts_path).ok();
            return fail(items, id, "Write error".to_string());
        }
        downloaded += bytes.len() as u64;
        let pct = 15.0 + 85.0 * (i + 1) as f64 / count as f64;
        with_item(items, id, |it| { it.progress = pct; it.downloaded_bytes = downloaded; });
    }
    drop(file);

    if let Err(e) = fs::write(&playlist_path, sidecar_playlist(&ts_name, total_secs)) {
        fs::remove_file(&ts_path).ok();
        return fail(items, id, e.to_string());
    }
    update_item(items, id, |it| {
        it.status = DownloadStatus::Done;
        it.progress = 100.0;
        it.downloaded_bytes = downloaded;
        it.total_bytes = downloaded;
        it.file_path = ts_path.to_string_lossy().to_string();
    });
}

#[tauri::command]
pub async fn start_bulk_download(anime_id: String, anime_name: String, episodes: Vec<String>, mode: String, dm: State<'_, DownloadManager>) -> Result<String, String> {
    for ep in &episodes {
        start_download(anime_id.clone(), anime_name.clone(), ep.clone(), mode.clone(), dm.clone()).await.ok();
    }
    Ok(format!("{} downloads queued", episodes.len()))
}

#[tauri::command]
pub async fn get_downloads(dm: State<'_, DownloadManager>) -> Result<Vec<DownloadItem>, String> {
    Ok(dm.items.lock().unwrap().clone())
}

#[tauri::command]
pub async fn cancel_download(id: String, dm: State<'_, DownloadManager>) -> Result<(), String> {
    let tx = dm.cancel_tx.lock().unwrap().remove(&id);
    if let Some(tx) = tx { tx.send(()).await.ok(); }
    Ok(())
}

#[tauri::command]
pub async fn remove_download(id: String, dm: State<'_, DownloadManager>) -> Result<(), String> {
    let mut items = dm.items.lock().unwrap();
    if let Some(item) = items.iter().find(|i| i.id == id) {
        if let Ok(path) = resolve_download_file(&item.file_path) {
            fs::remove_file(path.with_extension("m3u8")).ok();
            fs::remove_file(path).ok();
        }
    }
    items.retain(|i| i.id != id);
    store::save(DOWNLOADS_FILE, &*items);
    Ok(())
}

#[tauri::command]
pub async fn open_download_folder() -> Result<(), String> {
    open::that(download_dir()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_file(path: String) -> Result<(), String> {
    let file = resolve_download_file(&path)?;
    open::that(file).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_characters() {
        assert_eq!(sanitize("a/b:c d"), "a_b_c d");
    }

    #[test]
    fn parses_media_playlist_segments() {
        let base = reqwest::Url::parse("https://cdn.example/a/index.m3u8").unwrap();
        let text = "#EXTM3U\n#EXTINF:4.5,\nseg1.ts\n#EXTINF:3.0,\nhttps://other.example/seg2.ts\n#EXT-X-ENDLIST\n";
        assert_eq!(
            parse_media_playlist(&base, text).unwrap(),
            vec![(4.5, "https://cdn.example/a/seg1.ts".to_string()), (3.0, "https://other.example/seg2.ts".to_string())]
        );
    }

    #[test]
    fn rejects_encrypted_and_fragmented_playlists() {
        let base = reqwest::Url::parse("https://cdn.example/index.m3u8").unwrap();
        let aes = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:4,\na.ts\n";
        let map = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:4,\na.m4s\n";
        assert!(parse_media_playlist(&base, aes).is_err());
        assert!(parse_media_playlist(&base, map).is_err());
        assert!(parse_media_playlist(&base, "#EXTM3U\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:4,\na.ts\n").is_ok());
    }

    #[test]
    fn picks_highest_bandwidth_variant() {
        let base = reqwest::Url::parse("https://cdn.example/master.m3u8").unwrap();
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1280x720\nhigh.m3u8\n";
        assert_eq!(pick_variant(&base, master).unwrap().as_str(), "https://cdn.example/high.m3u8");
        assert!(pick_variant(&base, "#EXTM3U\n#EXTINF:4,\na.ts\n").is_none());
    }

    #[test]
    fn check_within_rejects_files_outside_base() {
        let root = std::env::temp_dir().join(format!("anides-dl-{}", std::process::id()));
        let inside = root.join("show");
        let outside = std::env::temp_dir().join(format!("anides-out-{}", std::process::id()));
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let ok_file = inside.join("ep.mp4");
        let bad_file = outside.join("secret.txt");
        fs::write(&ok_file, b"x").unwrap();
        fs::write(&bad_file, b"x").unwrap();

        assert!(check_within(&root, &ok_file).is_ok());
        assert!(check_within(&root, &bad_file).is_err());

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&outside).ok();
    }
}
