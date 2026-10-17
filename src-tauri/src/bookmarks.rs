use serde::{Deserialize, Serialize};

use crate::store;

const BOOKMARKS_FILE: &str = "bookmarks.json";

#[derive(Serialize, Deserialize, Clone)]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub query: String,
    pub mode: String,
    pub country: String,
    pub sort: String,
}

#[tauri::command]
pub async fn save_bookmark(name: String, query: String, mode: String, country: String, sort: String) -> Result<(), String> {
    let id = format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
    store::update::<Vec<Bookmark>, _>(BOOKMARKS_FILE, |entries| entries.push(Bookmark { id, name, query, mode, country, sort }));
    Ok(())
}

#[tauri::command]
pub async fn get_bookmarks() -> Result<Vec<Bookmark>, String> {
    Ok(store::load(BOOKMARKS_FILE))
}

#[tauri::command]
pub async fn delete_bookmark(id: String) -> Result<(), String> {
    store::update::<Vec<Bookmark>, _>(BOOKMARKS_FILE, |entries| entries.retain(|b| b.id != id));
    Ok(())
}
