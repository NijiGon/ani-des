use serde::{Deserialize, Serialize};
use regex::Regex;

const GOGO_BASE: &str = "https://anitaku.pe";

#[derive(Serialize, Deserialize, Clone)]
pub struct FallbackSource {
    pub quality: String,
    pub url: String,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build().unwrap()
}

/// Search Gogoanime for an anime and get streaming URL for an episode
pub async fn get_fallback_sources(title: &str, episode: &str) -> Vec<FallbackSource> {
    // Search for the anime
    let search_url = format!("{}/search.html?keyword={}", GOGO_BASE, urlencoding::encode(title));
    let html = match client().get(&search_url).send().await {
        Ok(r) => match r.text().await { Ok(t) => t, Err(_) => return vec![] },
        Err(_) => return vec![],
    };

    // Extract first result slug
    let slug_re = Regex::new(r#"<a href="/category/([^"]+)""#).unwrap();
    let slug = match slug_re.captures(&html) {
        Some(c) => c[1].to_string(),
        None => return vec![],
    };

    // Get episode page
    let ep_url = format!("{}/{}-episode-{}", GOGO_BASE, slug, episode);
    let ep_html = match client().get(&ep_url).send().await {
        Ok(r) => match r.text().await { Ok(t) => t, Err(_) => return vec![] },
        Err(_) => return vec![],
    };

    // Extract embed URL
    let embed_re = Regex::new(r#"data-video="([^"]+)""#).unwrap();
    let embed_url = match embed_re.captures(&ep_html) {
        Some(c) => {
            let u = c[1].to_string();
            if u.starts_with("//") { format!("https:{}", u) } else { u }
        },
        None => return vec![],
    };

    // Fetch embed page to get direct sources
    let embed_html = match client().get(&embed_url).send().await {
        Ok(r) => match r.text().await { Ok(t) => t, Err(_) => return vec![] },
        Err(_) => return vec![],
    };

    let mut sources = Vec::new();

    // Try to find m3u8 or mp4 links
    let source_re = Regex::new(r#"(?:file|src|source)[\s:'"=]+['"]?(https?://[^'"&\s]+\.(?:m3u8|mp4)[^'"&\s]*)"#).unwrap();
    for cap in source_re.captures_iter(&embed_html) {
        sources.push(FallbackSource {
            quality: if cap[1].contains("m3u8") { "hls".to_string() } else { "mp4".to_string() },
            url: cap[1].to_string(),
        });
    }

    sources
}

#[tauri::command]
pub async fn get_fallback_episode(title: String, episode: String) -> Result<Vec<FallbackSource>, String> {
    Ok(get_fallback_sources(&title, &episode).await)
}
