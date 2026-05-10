use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub query: String,
    pub mode: String,
    pub country: String,
    pub sort: String,
}

fn bookmarks_path() -> PathBuf {
    let dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("ani-des");
    fs::create_dir_all(&dir).ok();
    dir.join("bookmarks.json")
}

fn load_all() -> Vec<Bookmark> {
    fs::read_to_string(bookmarks_path()).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_all(entries: &[Bookmark]) {
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        fs::write(bookmarks_path(), json).ok();
    }
}

#[tauri::command]
pub async fn save_bookmark(name: String, query: String, mode: String, country: String, sort: String) -> Result<(), String> {
    let mut entries = load_all();
    let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
    entries.push(Bookmark { id, name, query, mode, country, sort });
    save_all(&entries);
    Ok(())
}

#[tauri::command]
pub async fn get_bookmarks() -> Result<Vec<Bookmark>, String> {
    Ok(load_all())
}

#[tauri::command]
pub async fn delete_bookmark(id: String) -> Result<(), String> {
    let mut entries = load_all();
    entries.retain(|b| b.id != id);
    save_all(&entries);
    Ok(())
}
