use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct ConsumetSource {
    pub quality: String,
    pub url: String,
}

/// Disabled - returns empty. AllAnime is the sole streaming provider.
pub async fn get_consumet_sources(_title: &str, _episode: &str) -> Vec<ConsumetSource> {
    vec![]
}

#[tauri::command]
pub async fn consumet_get_sources(_title: String, _episode: String) -> Result<Vec<ConsumetSource>, String> {
    Ok(vec![])
}
