use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub id: String,
    pub name: String,
    pub episode: String,
    pub mode: String,
    pub progress: f64,
    pub timestamp: u64,
    #[serde(default)]
    pub tag: String, // watching, break, dropped, plan_to_watch, finished
    #[serde(default)]
    pub thumbnail: String,
}

fn history_path() -> PathBuf {
    let dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("ani-des");
    fs::create_dir_all(&dir).ok();
    dir.join("history.json")
}

fn load_all() -> Vec<HistoryEntry> {
    fs::read_to_string(history_path()).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_all(entries: &[HistoryEntry]) {
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        fs::write(history_path(), json).ok();
    }
}

#[tauri::command]
pub async fn save_history(id: String, name: String, episode: String, mode: String, progress: f64) -> Result<(), String> {
    let mut entries = load_all();
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    if let Some(existing) = entries.iter_mut().find(|e| e.id == id && e.mode == mode) {
        existing.episode = episode;
        existing.progress = progress;
        existing.timestamp = ts;
        if existing.tag.is_empty() { existing.tag = "watching".to_string(); }
    } else {
        entries.push(HistoryEntry { id, name, episode, mode, progress, timestamp: ts, tag: "watching".to_string(), thumbnail: String::new() });
    }
    save_all(&entries);
    Ok(())
}

#[tauri::command]
pub async fn get_history() -> Result<Vec<HistoryEntry>, String> {
    let mut entries = load_all();
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(entries)
}

#[tauri::command]
pub async fn get_history_by_tag(tag: String) -> Result<Vec<HistoryEntry>, String> {
    let mut entries = load_all();
    entries.retain(|e| e.tag == tag);
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(entries)
}

#[tauri::command]
pub async fn set_history_tag(id: String, mode: String, tag: String) -> Result<(), String> {
    let mut entries = load_all();
    if let Some(e) = entries.iter_mut().find(|e| e.id == id && e.mode == mode) {
        e.tag = tag;
    }
    save_all(&entries);
    Ok(())
}

#[tauri::command]
pub async fn set_history_thumbnail(id: String, mode: String, thumbnail: String) -> Result<(), String> {
    let mut entries = load_all();
    if let Some(e) = entries.iter_mut().find(|e| e.id == id && e.mode == mode) {
        e.thumbnail = thumbnail;
    }
    save_all(&entries);
    Ok(())
}

#[tauri::command]
pub async fn delete_history(id: String, mode: String) -> Result<(), String> {
    let mut entries = load_all();
    entries.retain(|e| !(e.id == id && e.mode == mode));
    save_all(&entries);
    Ok(())
}

#[tauri::command]
pub async fn get_all_tags() -> Result<Vec<String>, String> {
    Ok(vec!["watching".to_string(), "plan_to_watch".to_string(), "break".to_string(), "finished".to_string(), "dropped".to_string()])
}
