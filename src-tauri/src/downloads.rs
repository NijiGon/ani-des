use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;
use tokio::sync::mpsc;

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
    pub progress: f64, // 0-100
    pub file_path: String,
    pub error: Option<String>,
}

pub struct DownloadManager {
    pub items: Arc<Mutex<Vec<DownloadItem>>>,
    pub cancel_tx: Arc<Mutex<HashMap<String, mpsc::Sender<()>>>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self { items: Arc::new(Mutex::new(Vec::new())), cancel_tx: Arc::new(Mutex::new(HashMap::new())) }
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

#[tauri::command]
pub async fn start_download(anime_id: String, anime_name: String, episode: String, mode: String, dm: State<'_, DownloadManager>) -> Result<String, String> {
    let id = format!("{}_{}", anime_id, episode);
    {
        let items = dm.items.lock().unwrap();
        if items.iter().any(|i| i.id == id && (i.status == DownloadStatus::Downloading || i.status == DownloadStatus::Done)) {
            return Err("Already downloaded or in progress".to_string());
        }
    }

    let item = DownloadItem {
        id: id.clone(), anime_id: anime_id.clone(), anime_name: anime_name.clone(),
        episode: episode.clone(), mode: mode.clone(),
        status: DownloadStatus::Queued, progress: 0.0, file_path: String::new(), error: None,
    };

    { let mut items = dm.items.lock().unwrap(); items.retain(|i| i.id != id); items.push(item); }

    let items_arc = dm.items.clone();
    let cancel_arc = dm.cancel_tx.clone();
    let (tx, mut rx) = mpsc::channel::<()>(1);
    { cancel_arc.lock().unwrap().insert(id.clone(), tx); }

    tokio::spawn(async move {
        do_download(id, anime_id, anime_name, episode, mode, items_arc, &mut rx).await;
    });

    Ok("queued".to_string())
}

async fn do_download(id: String, anime_id: String, anime_name: String, episode: String, mode: String, items: Arc<Mutex<Vec<DownloadItem>>>, cancel: &mut mpsc::Receiver<()>) {
    // Update status to downloading
    update_item(&items, &id, |i| { i.status = DownloadStatus::Downloading; i.progress = 5.0; });

    // Get episode URL
    let links = match get_links_for_download(&anime_id, &episode, &mode).await {
        Ok(l) => l,
        Err(e) => { update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some(e); }); return; }
    };

    let url = match links.first() {
        Some(l) => l.clone(),
        None => { update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some("No sources".to_string()); }); return; }
    };

    update_item(&items, &id, |i| i.progress = 15.0);

    let dir = download_dir().join(sanitize(&anime_name));
    fs::create_dir_all(&dir).ok();
    let file_path = dir.join(format!("Episode_{}.mp4", episode));

    // Download the file
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build().unwrap();
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => { update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some(e.to_string()); }); return; }
    };

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut file = match fs::File::create(&file_path) {
        Ok(f) => f,
        Err(e) => { update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some(e.to_string()); }); return; }
    };

    use std::io::Write;
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        // Check cancel
        if cancel.try_recv().is_ok() {
            update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some("Cancelled".to_string()); });
            fs::remove_file(&file_path).ok();
            return;
        }
        match chunk_result {
            Ok(bytes) => {
                if file.write_all(&bytes).is_err() {
                    update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some("Write error".to_string()); });
                    return;
                }
                downloaded += bytes.len() as u64;
                let pct = if total > 0 { 15.0 + (downloaded as f64 / total as f64) * 85.0 } else { 50.0 };
                update_item(&items, &id, |i| i.progress = pct);
            }
            Err(e) => { update_item(&items, &id, |i| { i.status = DownloadStatus::Failed; i.error = Some(e.to_string()); }); return; }
        }
    }

    update_item(&items, &id, |i| { i.status = DownloadStatus::Done; i.progress = 100.0; i.file_path = file_path.to_string_lossy().to_string(); });
}

fn update_item(items: &Arc<Mutex<Vec<DownloadItem>>>, id: &str, f: impl FnOnce(&mut DownloadItem)) {
    if let Ok(mut list) = items.lock() {
        if let Some(item) = list.iter_mut().find(|i| i.id == id) { f(item); }
    }
}

async fn get_links_for_download(show_id: &str, episode: &str, mode: &str) -> Result<Vec<String>, String> {
    use crate::api::{build_client, default_headers, allanime_api, decode_provider_id, decrypt_tobeparsed, fetch_links};
    use reqwest::header::{HeaderValue, REFERER};
    use regex::Regex;

    let mode_str = mode;
    let episode_gql = r#"query ($showId: String!, $translationType: VaildTranslationTypeEnumType!, $episodeString: String!) { episode( showId: $showId translationType: $translationType episodeString: $episodeString ) { episodeString sourceUrls }}"#;
    let query_hash = "d405d0edd690624b66baba3068e0edc3ac90f1597d898a1ec8db4e5c43c00fec";
    let vars = serde_json::json!({"showId": show_id, "translationType": mode_str, "episodeString": episode});
    let ext = serde_json::json!({"persistedQuery":{"version":1,"sha256Hash": query_hash}});

    let client = build_client();
    let vars_str = vars.to_string();
    let ext_str = ext.to_string();
    let encoded_vars = urlencoding::encode(&vars_str);
    let encoded_ext = urlencoding::encode(&ext_str);
    let api_url = format!("{}/api?variables={}&extensions={}", allanime_api(), encoded_vars, encoded_ext);

    let mut headers = default_headers();
    headers.insert("Origin", HeaderValue::from_static("https://youtu-chan.com"));
    headers.insert(REFERER, HeaderValue::from_static("https://youtu-chan.com"));

    let resp = client.get(&api_url).headers(headers).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let api_resp = if resp.is_empty() || !resp.contains("tobeparsed") {
        let body = serde_json::json!({"variables": {"showId": show_id, "translationType": mode_str, "episodeString": episode}, "query": episode_gql});
        client.post(format!("{}/api", allanime_api()))
            .headers(default_headers()).header("Content-Type", "application/json")
            .json(&body).send().await.map_err(|e| e.to_string())?
            .text().await.map_err(|e| e.to_string())?
    } else { resp };

    let sources: Vec<(String, String)> = if api_resp.contains("tobeparsed") {
        Regex::new(r#""tobeparsed":"([^"]*)""#).unwrap().captures(&api_resp)
            .map(|cap| decrypt_tobeparsed(&cap[1])).unwrap_or_default()
    } else {
        Regex::new(r#""sourceUrl":"--([^"]*)".*?"sourceName":"([^"]*)""#).unwrap()
            .captures_iter(&api_resp).map(|c| (c[2].to_string(), c[1].to_string())).collect()
    };

    let futs: Vec<_> = sources.iter()
        .map(|(_, eid)| decode_provider_id(eid))
        .filter(|id| !id.is_empty())
        .map(|id| fetch_links(id)).collect();
    let all: Vec<String> = futures::future::join_all(futs).await.into_iter().flatten()
        .filter(|l| !l.url.contains(".m3u8")) // prefer direct mp4 for downloads
        .map(|l| l.url).collect();

    if all.is_empty() {
        // fallback: include m3u8
        let futs2: Vec<_> = sources.iter()
            .map(|(_, eid)| decode_provider_id(eid))
            .filter(|id| !id.is_empty())
            .map(|id| fetch_links(id)).collect();
        let all2: Vec<String> = futures::future::join_all(futs2).await.into_iter().flatten().map(|l| l.url).collect();
        return Ok(all2);
    }
    Ok(all)
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
    dm.items.lock().unwrap().retain(|i| i.id != id);
    Ok(())
}

#[tauri::command]
pub async fn open_download_folder() -> Result<(), String> {
    let dir = download_dir();
    open::that(dir).map_err(|e| e.to_string())
}
