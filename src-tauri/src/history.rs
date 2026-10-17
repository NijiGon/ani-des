use serde::{Deserialize, Serialize};

use crate::store;

const HISTORY_FILE: &str = "history.json";

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

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn newest_first(mut entries: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries
}

#[tauri::command]
pub async fn save_history(id: String, name: String, episode: String, mode: String, progress: f64) -> Result<(), String> {
    let ts = now_secs();
    store::update::<Vec<HistoryEntry>, _>(HISTORY_FILE, |entries| {
        if let Some(existing) = entries.iter_mut().find(|e| e.id == id && e.mode == mode) {
            existing.episode = episode;
            existing.progress = progress;
            existing.timestamp = ts;
            if existing.tag.is_empty() { existing.tag = "watching".to_string(); }
        } else {
            entries.push(HistoryEntry { id, name, episode, mode, progress, timestamp: ts, tag: "watching".to_string(), thumbnail: String::new() });
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn get_history() -> Result<Vec<HistoryEntry>, String> {
    Ok(newest_first(store::load(HISTORY_FILE)))
}

#[tauri::command]
pub async fn get_history_by_tag(tag: String) -> Result<Vec<HistoryEntry>, String> {
    let entries: Vec<HistoryEntry> = store::load(HISTORY_FILE);
    Ok(newest_first(entries.into_iter().filter(|e| e.tag == tag).collect()))
}

#[tauri::command]
pub async fn set_history_tag(id: String, mode: String, tag: String) -> Result<(), String> {
    store::update::<Vec<HistoryEntry>, _>(HISTORY_FILE, |entries| {
        if let Some(e) = entries.iter_mut().find(|e| e.id == id && e.mode == mode) { e.tag = tag; }
    });
    Ok(())
}

#[tauri::command]
pub async fn set_history_thumbnail(id: String, mode: String, thumbnail: String) -> Result<(), String> {
    store::update::<Vec<HistoryEntry>, _>(HISTORY_FILE, |entries| {
        if let Some(e) = entries.iter_mut().find(|e| e.id == id && e.mode == mode) { e.thumbnail = thumbnail; }
    });
    Ok(())
}

#[tauri::command]
pub async fn delete_history(id: String, mode: String) -> Result<(), String> {
    store::update::<Vec<HistoryEntry>, _>(HISTORY_FILE, |entries| {
        entries.retain(|e| !(e.id == id && e.mode == mode));
    });
    Ok(())
}

#[tauri::command]
pub async fn get_all_tags() -> Result<Vec<String>, String> {
    Ok(vec!["watching".to_string(), "plan_to_watch".to_string(), "break".to_string(), "finished".to_string(), "dropped".to_string()])
}
