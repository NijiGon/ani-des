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
    let Some(url) = links.into_iter().next() else {
        return fail(&items, &id, "No direct video source (HLS-only streams cannot be downloaded yet)".to_string());
    };

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
    Ok(links.into_iter().map(|l| l.url).filter(|u| !u.contains(".m3u8")).collect())
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
        if let Ok(path) = resolve_download_file(&item.file_path) { fs::remove_file(path).ok(); }
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
