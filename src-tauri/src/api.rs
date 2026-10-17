use aes::cipher::{KeyIvInit, StreamCipher};
use base64::{engine::general_purpose::STANDARD, Engine};
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tauri::State;

use crate::jikan::{jikan_get, JIKAN_BASE};
use crate::store;

type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

const AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0";
const ALLANIME_REFR: &str = "https://allmanga.to";
const ALLANIME_BASE: &str = "allanime.day";
const ALLANIME_KEY_INPUT: &str = "Xot36i3lK3:v1";
const EPISODE_QUERY_HASH: &str = "d405d0edd690624b66baba3068e0edc3ac90f1597d898a1ec8db4e5c43c00fec";
const EPISODE_GQL: &str = r#"query ($showId: String!, $translationType: VaildTranslationTypeEnumType!, $episodeString: String!) { episode( showId: $showId translationType: $translationType episodeString: $episodeString ) { episodeString sourceUrls }}"#;
const MAL_MAP_FILE: &str = "mal_mappings.json";
const CACHE_TTL_SECS: u64 = 300; // 5 minutes

pub fn allanime_api() -> String { format!("https://api.{}", ALLANIME_BASE) }

fn derive_key() -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(ALLANIME_KEY_INPUT.as_bytes());
    h.finalize().to_vec()
}

pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

pub fn default_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(USER_AGENT, HeaderValue::from_static(AGENT));
    h.insert(REFERER, HeaderValue::from_static(ALLANIME_REFR));
    h
}

/// Longest prefix of `s` that is at most `n` bytes and ends on a char boundary.
pub fn snippet(s: &str, n: usize) -> &str {
    let mut end = n.min(s.len());
    while !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

fn mode_str(mode: &str) -> &'static str {
    if mode == "dub" { "dub" } else { "sub" }
}

// --- Cache (disk-based) ---
pub struct AppCache { dir: PathBuf }

impl AppCache {
    pub fn new() -> Self {
        let dir = store::data_dir().join("cache");
        fs::create_dir_all(&dir).ok();
        Self { dir }
    }
}

fn cache_path(cache: &AppCache, prefix: &str, key: &str) -> PathBuf {
    let mut h = Sha256::new(); h.update(key.as_bytes());
    let hash = format!("{:x}", h.finalize());
    cache.dir.join(format!("{}_{}.json", prefix, &hash[..16]))
}

fn disk_cache_get<T: serde::de::DeserializeOwned>(cache: &AppCache, prefix: &str, key: &str) -> Option<T> {
    let path = cache_path(cache, prefix, key);
    let meta = fs::metadata(&path).ok()?;
    if meta.modified().ok()?.elapsed().ok()? > Duration::from_secs(CACHE_TTL_SECS) {
        fs::remove_file(&path).ok();
        return None;
    }
    let data = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

fn prune_expired(cache: &AppCache) {
    let Ok(entries) = fs::read_dir(&cache.dir) else { return };
    for e in entries.flatten() {
        let expired = e.metadata().and_then(|m| m.modified()).ok()
            .and_then(|t| t.elapsed().ok())
            .map_or(false, |age| age > Duration::from_secs(CACHE_TTL_SECS));
        if expired { fs::remove_file(e.path()).ok(); }
    }
}

fn disk_cache_set<T: Serialize>(cache: &AppCache, prefix: &str, key: &str, data: &T) {
    let Ok(value) = serde_json::to_value(data) else { return };
    // Never cache empty results: a failed lookup would otherwise hide retries for the whole TTL.
    if value.as_array().map_or(false, |a| a.is_empty()) { return; }
    prune_expired(cache);
    if let Ok(json) = serde_json::to_string(&value) {
        fs::write(cache_path(cache, prefix, key), json).ok();
    }
}

/// Stores show id -> MAL id pairs in one locked write.
fn mal_map_save(pairs: Vec<(String, u64)>) {
    let pairs: Vec<_> = pairs.into_iter().filter(|(_, m)| *m != 0).collect();
    if pairs.is_empty() { return; }
    store::update::<HashMap<String, u64>, _>(MAL_MAP_FILE, |map| map.extend(pairs));
}

#[tauri::command]
pub async fn get_mal_id(show_id: String) -> Result<u64, String> {
    let map: HashMap<String, u64> = store::load(MAL_MAP_FILE);
    map.get(&show_id).copied().ok_or_else(|| "Not found".to_string())
}

// --- Types ---
#[derive(Serialize, Deserialize, Clone)]
pub struct AnimeResult { pub id: String, pub name: String, pub episodes: u32, pub thumbnail: String, pub rating: f64, #[serde(default)] pub mal_id: u64 }

#[derive(Serialize, Deserialize, Clone)]
pub struct EpisodeLink { pub quality: String, pub url: String }

#[derive(Serialize, Deserialize, Clone)]
pub struct CacheStats { pub search_entries: usize, pub episode_entries: usize, pub link_entries: usize }

// --- Decoding ---
pub fn decode_provider_id(encoded: &str) -> String {
    let map: &[(&str, &str)] = &[
        ("79","A"),("7a","B"),("7b","C"),("7c","D"),("7d","E"),("7e","F"),("7f","G"),
        ("70","H"),("71","I"),("72","J"),("73","K"),("74","L"),("75","M"),("76","N"),
        ("77","O"),("68","P"),("69","Q"),("6a","R"),("6b","S"),("6c","T"),("6d","U"),
        ("6e","V"),("6f","W"),("60","X"),("61","Y"),("62","Z"),("59","a"),("5a","b"),
        ("5b","c"),("5c","d"),("5d","e"),("5e","f"),("5f","g"),("50","h"),("51","i"),
        ("52","j"),("53","k"),("54","l"),("55","m"),("56","n"),("57","o"),("48","p"),
        ("49","q"),("4a","r"),("4b","s"),("4c","t"),("4d","u"),("4e","v"),("4f","w"),
        ("40","x"),("41","y"),("42","z"),("08","0"),("09","1"),("0a","2"),("0b","3"),
        ("0c","4"),("0d","5"),("0e","6"),("0f","7"),("00","8"),("01","9"),("15","-"),
        ("16","."),("67","_"),("46","~"),("02",":"),("17","/"),("07","?"),("1b","#"),
        ("63","["),("65","]"),("78","@"),("19","!"),("1c","$"),("1e","&"),("10","("),
        ("11",")"),("12","*"),("13","+"),("14",","),("03",";"),("05","="),("1d","%"),
    ];
    let mut result = String::new();
    let chars: Vec<char> = encoded.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        let pair = format!("{}{}", chars[i], chars[i + 1]);
        if let Some((_, decoded)) = map.iter().find(|(k, _)| *k == pair.as_str()) {
            result.push_str(decoded);
        }
        i += 2;
    }
    result.replace("/clock", "/clock.json")
}

pub fn decrypt_tobeparsed(blob: &str) -> Vec<(String, String)> {
    let data = match STANDARD.decode(blob) { Ok(d) => d, Err(_) => return vec![] };
    if data.len() < 29 { return vec![]; }
    let iv = &data[1..13];
    let ct = &data[13..data.len() - 16];
    let key = derive_key();
    let mut ctr_iv = [0u8; 16];
    ctr_iv[..12].copy_from_slice(iv);
    ctr_iv[15] = 2;
    let mut buf = ct.to_vec();
    let mut cipher = Aes256Ctr::new(key.as_slice().into(), &ctr_iv.into());
    cipher.apply_keystream(&mut buf);
    let plain = String::from_utf8_lossy(&buf).to_string();
    let re = Regex::new(r#""sourceUrl":"--([^"]*)".*?"sourceName":"([^"]*)""#).unwrap();
    re.captures_iter(&plain).map(|c| (c[2].to_string(), c[1].to_string())).collect()
}

/// Extracts (source name, encoded provider id) pairs from an AllAnime episode response.
pub fn parse_sources(api_resp: &str) -> Vec<(String, String)> {
    if api_resp.contains("tobeparsed") {
        return Regex::new(r#""tobeparsed":"([^"]*)""#).unwrap().captures(api_resp)
            .map(|cap| decrypt_tobeparsed(&cap[1])).unwrap_or_default();
    }
    let mut s: Vec<(String, String)> = Regex::new(r#""sourceUrl":"--([^"]*)".*?"sourceName":"([^"]*)""#).unwrap()
        .captures_iter(api_resp).map(|c| (c[2].to_string(), c[1].to_string())).collect();
    if s.is_empty() {
        // Alternate order: sourceName before sourceUrl
        s = Regex::new(r#""sourceName":"([^"]*)".*?"sourceUrl":"--([^"]*)""#).unwrap()
            .captures_iter(api_resp).map(|c| (c[1].to_string(), c[2].to_string())).collect();
    }
    s
}

pub async fn fetch_links(provider_id: String) -> Vec<EpisodeLink> {
    // If the provider_id is already a full URL (e.g. tools.fast4speed.rsvp), it IS the video link
    if provider_id.starts_with("http") {
        return vec![EpisodeLink { quality: "default".to_string(), url: provider_id }];
    }

    let client = build_client();
    let url = format!("https://{}{}", ALLANIME_BASE, provider_id);
    let body = match client.get(&url).headers(default_headers()).send().await {
        Ok(r) => match r.text().await { Ok(t) => t, Err(_) => return vec![] },
        Err(_) => return vec![],
    };

    let mut links = Vec::new();

    // Format 1: {"link":"...","resolutionStr":"..."}
    let re = Regex::new(r#""link"\s*:\s*"([^"]*)".*?"resolutionStr"\s*:\s*"([^"]*)""#).unwrap();
    for cap in re.captures_iter(&body) {
        links.push(EpisodeLink { quality: cap[2].to_string(), url: cap[1].to_string() });
    }

    // Format 2: {"resolutionStr":"...","link":"..."}  (reversed order)
    if links.is_empty() {
        let re2 = Regex::new(r#""resolutionStr"\s*:\s*"([^"]*)".*?"link"\s*:\s*"([^"]*)""#).unwrap();
        for cap in re2.captures_iter(&body) {
            links.push(EpisodeLink { quality: cap[1].to_string(), url: cap[2].to_string() });
        }
    }

    // Format 3: HLS with hardsub
    let re_hls = Regex::new(r#""hls"\s*,\s*"url"\s*:\s*"([^"]*)".*?"hardsub_lang"\s*:\s*"en-US""#).unwrap();
    for cap in re_hls.captures_iter(&body) {
        links.push(EpisodeLink { quality: "hls".to_string(), url: cap[1].to_string() });
    }

    // Format 4: Simple {"url":"..."} or {"file":"..."}
    if links.is_empty() {
        let re3 = Regex::new(r#""(?:url|file)"\s*:\s*"(https?://[^"]*)""#).unwrap();
        for cap in re3.captures_iter(&body) {
            links.push(EpisodeLink { quality: "default".to_string(), url: cap[1].to_string() });
        }
    }

    // Format 5: m3u8 URL anywhere in response
    if links.is_empty() {
        let re4 = Regex::new(r#"(https?://[^"]*\.m3u8[^"]*)"#).unwrap();
        for cap in re4.captures_iter(&body) {
            links.push(EpisodeLink { quality: "hls".to_string(), url: cap[1].to_string() });
        }
    }

    links
}

/// Resolves every provider in `sources` to its links, concurrently.
pub async fn resolve_sources(sources: &[(String, String)]) -> Vec<EpisodeLink> {
    let futs: Vec<_> = sources.iter()
        .map(|(_, eid)| decode_provider_id(eid))
        .filter(|id| !id.is_empty())
        .map(|id| fetch_links(id)).collect();
    futures::future::join_all(futs).await.into_iter().flatten().collect()
}

/// Raw responses from the AllAnime episode lookup, kept for debugging.
pub struct SourceFetch {
    pub get_url: String,
    pub get_resp: String,
    pub post_resp: Option<String>,
    /// The response that was actually parsed (GET, or POST when GET had no sources).
    pub api_resp: String,
}

/// GET the persisted-query episode lookup, falling back to POST when GET has no sources.
pub async fn fetch_source_list(show_id: &str, episode: &str, mode: &str) -> Result<SourceFetch, String> {
    let vars = json!({"showId": show_id, "translationType": mode, "episodeString": episode});
    let ext = json!({"persistedQuery": {"version": 1, "sha256Hash": EPISODE_QUERY_HASH}});

    // Match ani-cli's minimal encoding: only encode " : { } ,
    let encoded_vars = vars.to_string().replace('"', "%22").replace(':', "%3A").replace('{', "%7B").replace('}', "%7D").replace(',', "%2C");
    let encoded_ext = ext.to_string().replace('"', "%22").replace(':', "%3A").replace('{', "%7B").replace('}', "%7D").replace(',', "%2C").replace(' ', "%20");
    let get_url = format!("{}/api?variables={}&extensions={}", allanime_api(), encoded_vars, encoded_ext);

    let mut headers = default_headers();
    headers.insert("Origin", HeaderValue::from_static("https://youtu-chan.com"));
    headers.insert(REFERER, HeaderValue::from_static("https://youtu-chan.com"));

    let client = build_client();
    let get_resp = client.get(&get_url).headers(headers).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;
    if !get_resp.is_empty() && get_resp.contains("tobeparsed") {
        return Ok(SourceFetch { get_url, api_resp: get_resp.clone(), get_resp, post_resp: None });
    }

    crate::logging::log("GET had no tobeparsed, trying POST fallback");
    let body = json!({"variables": {"showId": show_id, "translationType": mode, "episodeString": episode}, "query": EPISODE_GQL});
    let post_resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;
    crate::logging::log(&format!("POST response: {} bytes - {}", post_resp.len(), snippet(&post_resp, 300)));
    Ok(SourceFetch { get_url, get_resp, api_resp: post_resp.clone(), post_resp: Some(post_resp) })
}

// --- Commands ---
#[tauri::command]
pub async fn search_anime(query: String, mode: String, country: Option<String>, cache: State<'_, AppCache>) -> Result<Vec<AnimeResult>, String> {
    let mode = mode_str(&mode);
    let country = country.unwrap_or_else(|| "ALL".to_string());
    let cache_key = format!("{}:{}:{}", query, mode, country);

    if let Some(cached) = disk_cache_get::<Vec<AnimeResult>>(&cache, "search", &cache_key) { return Ok(cached); }

    let search_gql = r#"query( $search: SearchInput $limit: Int $page: Int $translationType: VaildTranslationTypeEnumType $countryOrigin: VaildCountryOriginEnumType ) { shows( search: $search limit: $limit page: $page translationType: $translationType countryOrigin: $countryOrigin ) { edges { _id name thumbnail score malId availableEpisodes __typename } }}"#;

    let body = json!({
        "variables": {
            "search": {"allowAdult": false, "allowUnknown": false, "query": query},
            "limit": 40, "page": 1,
            "translationType": mode,
            "countryOrigin": country
        },
        "query": search_gql
    });

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    let mut mal_pairs = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
        if let Some(edges) = json.pointer("/data/shows/edges").and_then(|v| v.as_array()) {
            for edge in edges {
                let id = edge.get("_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let name = edge.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let thumbnail = edge.get("thumbnail").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let episodes = edge.pointer(&format!("/availableEpisodes/{}", mode))
                    .and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let rating = edge.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let mal_id = edge.get("malId").and_then(|v| v.as_u64()).unwrap_or(0);
                if episodes > 0 {
                    mal_pairs.push((id.clone(), mal_id));
                    results.push(AnimeResult { id, name, episodes, thumbnail, rating, mal_id });
                }
            }
        }
    }
    mal_map_save(mal_pairs);
    disk_cache_set(&cache, "search", &cache_key, &results);
    Ok(results)
}

#[tauri::command]
pub async fn get_episodes(show_id: String, mode: String, cache: State<'_, AppCache>) -> Result<Vec<String>, String> {
    let mode = mode_str(&mode);
    let cache_key = format!("{}:{}", show_id, mode);

    if let Some(cached) = disk_cache_get::<Vec<String>>(&cache, "episodes", &cache_key) { return Ok(cached); }

    let gql = r#"query ($showId: String!) { show( _id: $showId ) { _id availableEpisodesDetail }}"#;
    let body = json!({"variables": {"showId": show_id}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let pattern = format!(r#""{}":\[([^\]]*)\]"#, mode);
    let re = Regex::new(&pattern).unwrap();
    let eps = if let Some(cap) = re.captures(&resp) {
        let mut eps: Vec<String> = cap[1].split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect();
        eps.sort_by(|a, b| a.parse::<f64>().unwrap_or(0.0).partial_cmp(&b.parse::<f64>().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
        eps
    } else { vec![] };

    disk_cache_set(&cache, "episodes", &cache_key, &eps);
    Ok(eps)
}

#[tauri::command]
pub async fn get_episode_url(show_id: String, episode: String, mode: String, quality: String, title: Option<String>, cache: State<'_, AppCache>) -> Result<Vec<EpisodeLink>, String> {
    let mode = mode_str(&mode);
    let cache_key = format!("{}:{}:{}", show_id, episode, mode);

    if let Some(cached) = disk_cache_get::<Vec<EpisodeLink>>(&cache, "links", &cache_key) { return Ok(cached); }

    let fetch = fetch_source_list(&show_id, &episode, mode).await?;
    crate::logging::log(&format!("GET {} -> {} bytes", fetch.get_url, fetch.get_resp.len()));
    crate::logging::log(&format!("GET response: {}", snippet(&fetch.get_resp, 300)));

    let sources = parse_sources(&fetch.api_resp);
    crate::logging::log(&format!("Sources found: {} | has_tobeparsed: {} | providers: {:?}", sources.len(), fetch.api_resp.contains("tobeparsed"), sources.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()));
    for (name, eid) in &sources {
        crate::logging::log(&format!("  Provider '{}' encoded_id: {}... -> decoded: {}", name, snippet(eid, 30), snippet(&decode_provider_id(eid), 60)));
    }

    let all_links = resolve_sources(&sources).await;
    crate::logging::log(&format!("Total links fetched: {}", all_links.len()));

    if !quality.is_empty() && quality != "best" {
        let filtered: Vec<EpisodeLink> = all_links.iter().filter(|l| l.quality.contains(&quality)).cloned().collect();
        if !filtered.is_empty() {
            disk_cache_set(&cache, "links", &cache_key, &filtered);
            return Ok(filtered);
        }
    }

    // Fallback to Gogoanime if AllAnime returned nothing
    if all_links.is_empty() {
        let search_title = title.unwrap_or_else(|| show_id.clone());
        let gogo_sources = crate::consumet::get_fallback_sources(&search_title, &episode).await;
        let fallback: Vec<EpisodeLink> = gogo_sources.into_iter()
            .map(|s| EpisodeLink { quality: format!("gogo-{}", s.quality), url: s.url })
            .collect();
        if !fallback.is_empty() {
            disk_cache_set(&cache, "links", &cache_key, &fallback);
            return Ok(fallback);
        }
    }

    disk_cache_set(&cache, "links", &cache_key, &all_links);
    Ok(all_links)
}

#[tauri::command]
pub async fn get_cache_stats(cache: State<'_, AppCache>) -> Result<CacheStats, String> {
    let mut stats = CacheStats { search_entries: 0, episode_entries: 0, link_entries: 0 };
    for e in fs::read_dir(&cache.dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("search_") { stats.search_entries += 1; }
        else if name.starts_with("episodes_") { stats.episode_entries += 1; }
        else if name.starts_with("links_") { stats.link_entries += 1; }
    }
    Ok(stats)
}

#[tauri::command]
pub async fn clear_cache(cache: State<'_, AppCache>) -> Result<(), String> {
    if let Ok(entries) = fs::read_dir(&cache.dir) {
        for e in entries.flatten() { fs::remove_file(e.path()).ok(); }
    }
    Ok(())
}


#[derive(Serialize, Deserialize, Clone)]
pub struct AnimeDetails {
    pub id: String,
    pub name: String,
    pub description: String,
    pub thumbnail: String,
    pub genres: Vec<String>,
    pub status: String,
    pub episode_count: u32,
    pub rating: f64,
}

#[tauri::command]
pub async fn get_anime_details(show_id: String) -> Result<AnimeDetails, String> {
    let gql = r#"query ($showId: String!) { show( _id: $showId ) { _id name thumbnail description genres status score availableEpisodes }}"#;
    let body = json!({"variables": {"showId": show_id}, "query": gql});

    let client = build_client();
    let json: serde_json::Value = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    let show = json.pointer("/data/show").ok_or_else(|| "Not found".to_string())?;
    let text = |key: &str| show.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let genres = show.get("genres").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|g| g.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    Ok(AnimeDetails {
        id: show_id,
        name: text("name"),
        description: text("description"),
        thumbnail: text("thumbnail"),
        genres,
        status: show.get("status").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string(),
        episode_count: show.pointer("/availableEpisodes/sub").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        rating: show.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
    })
}

#[tauri::command]
pub async fn get_popular(mode: String, page: Option<u32>, cache: State<'_, AppCache>) -> Result<Vec<AnimeResult>, String> {
    let mode = mode_str(&mode);
    let pg = page.unwrap_or(1);
    let cache_key = format!("popular:{}:{}", mode, pg);
    if let Some(cached) = disk_cache_get::<Vec<AnimeResult>>(&cache, "search", &cache_key) { return Ok(cached); }

    let gql = r#"query( $type: VaildShowObjectEnumType $page: Int $limit: Int $translationType: VaildTranslationTypeEnumType ) { queryPopular( type: $type page: $page limit: $limit translationType: $translationType ) { recommendations { anyCard { _id name thumbnail score malId availableEpisodes } } }}"#;
    let body = json!({"variables": {"type": "anime", "page": pg, "limit": 40, "translationType": mode}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    let mut mal_pairs = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
        if let Some(recs) = json.pointer("/data/queryPopular/recommendations").and_then(|v| v.as_array()) {
            for rec in recs {
                if let Some(card) = rec.get("anyCard") {
                    let id = card.get("_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let name = card.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let thumbnail = card.get("thumbnail").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    let rating = card.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let mal_id = card.get("malId").and_then(|v| v.as_u64()).unwrap_or(0);
                    let episodes = card.pointer(&format!("/availableEpisodes/{}", mode)).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    if !id.is_empty() {
                        mal_pairs.push((id.clone(), mal_id));
                        results.push(AnimeResult { id, name, episodes, thumbnail, rating, mal_id });
                    }
                }
            }
        }
    }
    mal_map_save(mal_pairs);
    disk_cache_set(&cache, "search", &cache_key, &results);
    Ok(results)
}

#[tauri::command]
pub async fn get_recently_updated(mode: String, page: Option<u32>, cache: State<'_, AppCache>) -> Result<Vec<AnimeResult>, String> {
    let mode = mode_str(&mode);
    let pg = page.unwrap_or(1);
    let cache_key = format!("recent:{}:{}", mode, pg);
    if let Some(cached) = disk_cache_get::<Vec<AnimeResult>>(&cache, "search", &cache_key) { return Ok(cached); }

    let gql = r#"query( $search: SearchInput $limit: Int $page: Int $translationType: VaildTranslationTypeEnumType $countryOrigin: VaildCountryOriginEnumType ) { shows( search: $search limit: $limit page: $page translationType: $translationType countryOrigin: $countryOrigin ) { edges { _id name thumbnail score malId availableEpisodes __typename } }}"#;
    let body = json!({"variables": {"search": {"allowAdult": false, "allowUnknown": false, "query": ""}, "limit": 40, "page": pg, "translationType": mode, "countryOrigin": "ALL"}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    let mut mal_pairs = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
        if let Some(edges) = json.pointer("/data/shows/edges").and_then(|v| v.as_array()) {
            for edge in edges {
                let id = edge.get("_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let name = edge.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let thumbnail = edge.get("thumbnail").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let rating = edge.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let mal_id = edge.get("malId").and_then(|v| v.as_u64()).unwrap_or(0);
                let episodes = edge.pointer(&format!("/availableEpisodes/{}", mode)).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if episodes > 0 {
                    mal_pairs.push((id.clone(), mal_id));
                    results.push(AnimeResult { id, name, episodes, thumbnail, rating, mal_id });
                }
            }
        }
    }
    mal_map_save(mal_pairs);
    disk_cache_set(&cache, "search", &cache_key, &results);
    Ok(results)
}

#[derive(Serialize, Clone)]
pub struct DebugInfo {
    pub get_url: String,
    pub get_response_len: usize,
    pub get_response_snippet: String,
    pub used_post_fallback: bool,
    pub post_response_len: usize,
    pub post_response_snippet: String,
    pub has_tobeparsed: bool,
    pub sources_count: usize,
    pub sources: Vec<(String, String)>,
    pub decoded_provider_ids: Vec<String>,
    pub links_per_provider: Vec<(String, usize)>,
}

#[tauri::command]
pub async fn debug_episode_url(show_id: String, episode: String, mode: String) -> Result<DebugInfo, String> {
    let fetch = fetch_source_list(&show_id, &episode, mode_str(&mode)).await?;
    let sources = parse_sources(&fetch.api_resp);

    let mut info = DebugInfo {
        get_url: fetch.get_url.clone(),
        get_response_len: fetch.get_resp.len(),
        get_response_snippet: snippet(&fetch.get_resp, 500).to_string(),
        used_post_fallback: fetch.post_resp.is_some(),
        post_response_len: fetch.post_resp.as_ref().map_or(0, |p| p.len()),
        post_response_snippet: fetch.post_resp.as_ref().map_or(String::new(), |p| snippet(p, 500).to_string()),
        has_tobeparsed: fetch.api_resp.contains("tobeparsed"),
        sources_count: sources.len(),
        sources: sources.clone(),
        decoded_provider_ids: vec![],
        links_per_provider: vec![],
    };

    for (name, eid) in &sources {
        let pid = decode_provider_id(eid);
        info.decoded_provider_ids.push(pid.clone());
        if !pid.is_empty() {
            let links = fetch_links(pid).await;
            info.links_per_provider.push((name.clone(), links.len()));
        }
    }

    Ok(info)
}

#[tauri::command]
pub async fn get_thumbnail_fallback(name: String, mal_id: Option<u64>) -> Result<String, String> {
    // If we have MAL ID, use it directly (faster, more accurate)
    if let Some(id) = mal_id.filter(|&id| id > 0) {
        if let Ok(json) = jikan_get(&format!("{}/anime/{}", JIKAN_BASE, id)).await {
            if let Some(img) = json.pointer("/data/images/jpg/large_image_url").and_then(|v| v.as_str()) {
                return Ok(img.to_string());
            }
        }
    }
    // Fallback: search by name
    let url = format!("{}/anime?q={}&limit=1", JIKAN_BASE, urlencoding::encode(&name));
    let json = jikan_get(&url).await?;
    json.pointer("/data/0/images/jpg/large_image_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No thumbnail found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_provider_ids() {
        assert_eq!(decode_provider_id("7a7b"), "BC");
        assert_eq!(decode_provider_id("0809"), "01");
        assert_eq!(decode_provider_id("4b4b"), "ss");
    }

    #[test]
    fn snippet_respects_char_boundaries() {
        assert_eq!(snippet("あいう", 4), "あ");
        assert_eq!(snippet("abc", 10), "abc");
    }

    #[test]
    fn parses_plain_sources_in_both_orders() {
        let a = r#"{"sourceUrl":"--79","sourceName":"Default"}"#;
        assert_eq!(parse_sources(a), vec![("Default".to_string(), "79".to_string())]);
        let b = r#"{"sourceName":"Luf","sourceUrl":"--7a"}"#;
        assert_eq!(parse_sources(b), vec![("Luf".to_string(), "7a".to_string())]);
    }

    #[test]
    fn decrypts_tobeparsed_blob() {
        let plain = r#""sourceUrl":"--79","sourceName":"Default""#;
        let iv = [7u8; 12];
        let mut ctr_iv = [0u8; 16];
        ctr_iv[..12].copy_from_slice(&iv);
        ctr_iv[15] = 2;
        let mut ct = plain.as_bytes().to_vec();
        let mut cipher = Aes256Ctr::new(derive_key().as_slice().into(), &ctr_iv.into());
        cipher.apply_keystream(&mut ct);

        let mut blob = vec![0u8];
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(&ct);
        blob.extend_from_slice(&[0u8; 16]);
        let encoded = STANDARD.encode(&blob);
        assert_eq!(decrypt_tobeparsed(&encoded), vec![("Default".to_string(), "79".to_string())]);
    }
}
