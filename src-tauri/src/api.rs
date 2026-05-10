use aes::cipher::{KeyIvInit, StreamCipher};
use base64::{engine::general_purpose::STANDARD, Engine};
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::State;

type Aes256Ctr = ctr::Ctr128BE<aes::Aes256>;

const AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0";
const ALLANIME_REFR: &str = "https://allmanga.to";
const ALLANIME_BASE: &str = "allanime.day";
const ALLANIME_KEY_INPUT: &str = "Xot36i3lK3:v1";
const CACHE_TTL_SECS: u64 = 300; // 5 minutes

pub fn allanime_api() -> String { format!("https://api.{}", ALLANIME_BASE) }

fn derive_key() -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(ALLANIME_KEY_INPUT.as_bytes());
    h.finalize().to_vec()
}

pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
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

// --- Cache ---
struct CacheEntry<T> { data: T, created: Instant }

pub struct AppCache {
    search: Mutex<HashMap<String, CacheEntry<Vec<AnimeResult>>>>,
    episodes: Mutex<HashMap<String, CacheEntry<Vec<String>>>>,
    links: Mutex<HashMap<String, CacheEntry<Vec<EpisodeLink>>>>,
}

impl AppCache {
    pub fn new() -> Self {
        Self {
            search: Mutex::new(HashMap::new()),
            episodes: Mutex::new(HashMap::new()),
            links: Mutex::new(HashMap::new()),
        }
    }
}

fn cache_valid<T>(cache: &Mutex<HashMap<String, CacheEntry<T>>>, key: &str) -> Option<T> where T: Clone {
    let map = cache.lock().ok()?;
    let entry = map.get(key)?;
    if entry.created.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
        Some(entry.data.clone())
    } else { None }
}

fn cache_set<T: Clone>(cache: &Mutex<HashMap<String, CacheEntry<T>>>, key: String, data: T) {
    if let Ok(mut map) = cache.lock() {
        map.insert(key, CacheEntry { data, created: Instant::now() });
    }
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

// --- Commands ---
#[tauri::command]
pub async fn search_anime(query: String, mode: String, country: Option<String>, cache: State<'_, AppCache>) -> Result<Vec<AnimeResult>, String> {
    let mode = if mode == "dub" { "dub" } else { "sub" };
    let country = country.unwrap_or_else(|| "ALL".to_string());
    let cache_key = format!("{}:{}:{}", query, mode, country);

    if let Some(cached) = cache_valid(&cache.search, &cache_key) { return Ok(cached); }

    let search_gql = r#"query( $search: SearchInput $limit: Int $page: Int $translationType: VaildTranslationTypeEnumType $countryOrigin: VaildCountryOriginEnumType ) { shows( search: $search limit: $limit page: $page translationType: $translationType countryOrigin: $countryOrigin ) { edges { _id name thumbnail score malId availableEpisodes __typename } }}"#;

    let body = serde_json::json!({
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
                    results.push(AnimeResult { id, name, episodes, thumbnail, rating, mal_id });
                }
            }
        }
    }
    cache_set(&cache.search, cache_key, results.clone());
    Ok(results)
}

#[tauri::command]
pub async fn get_episodes(show_id: String, mode: String, cache: State<'_, AppCache>) -> Result<Vec<String>, String> {
    let mode = if mode == "dub" { "dub" } else { "sub" };
    let cache_key = format!("{}:{}", show_id, mode);

    if let Some(cached) = cache_valid(&cache.episodes, &cache_key) { return Ok(cached); }

    let gql = r#"query ($showId: String!) { show( _id: $showId ) { _id availableEpisodesDetail }}"#;
    let body = serde_json::json!({"variables": {"showId": show_id}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let pattern = format!(r#""{}":\[([^\]]*)\]"#, mode);
    let re = Regex::new(&pattern).unwrap();
    let eps = if let Some(cap) = re.captures(&resp) {
        let mut eps: Vec<String> = cap[1].split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect();
        eps.sort_by(|a, b| a.parse::<f64>().unwrap_or(0.0).partial_cmp(&b.parse::<f64>().unwrap_or(0.0)).unwrap());
        eps
    } else { vec![] };

    cache_set(&cache.episodes, cache_key, eps.clone());
    Ok(eps)
}

#[tauri::command]
pub async fn get_episode_url(show_id: String, episode: String, mode: String, quality: String, title: Option<String>, cache: State<'_, AppCache>) -> Result<Vec<EpisodeLink>, String> {
    let mode_str = if mode == "dub" { "dub" } else { "sub" };
    let cache_key = format!("{}:{}:{}", show_id, episode, mode_str);

    if let Some(cached) = cache_valid(&cache.links, &cache_key) { return Ok(cached); }

    let episode_gql = r#"query ($showId: String!, $translationType: VaildTranslationTypeEnumType!, $episodeString: String!) { episode( showId: $showId translationType: $translationType episodeString: $episodeString ) { episodeString sourceUrls }}"#;
    let query_hash = "d405d0edd690624b66baba3068e0edc3ac90f1597d898a1ec8db4e5c43c00fec";
    let vars = serde_json::json!({"showId": show_id, "translationType": mode_str, "episodeString": episode});
    let ext = serde_json::json!({"persistedQuery":{"version":1,"sha256Hash": query_hash}});

    let client = build_client();
    let vars_str = vars.to_string();
    let ext_str = ext.to_string();
    // Match ani-cli's minimal encoding: only encode " : { } ,
    let encoded_vars = vars_str.replace('"', "%22").replace(':', "%3A").replace('{', "%7B").replace('}', "%7D").replace(',', "%2C");
    let encoded_ext = ext_str.replace('"', "%22").replace(':', "%3A").replace('{', "%7B").replace('}', "%7D").replace(',', "%2C").replace(' ', "%20");
    let api_url = format!("{}/api?variables={}&extensions={}", allanime_api(), encoded_vars, encoded_ext);

    let mut headers = default_headers();
    headers.insert("Origin", HeaderValue::from_static("https://youtu-chan.com"));
    headers.insert(REFERER, HeaderValue::from_static("https://youtu-chan.com"));

    let resp = client.get(&api_url).headers(headers).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    crate::logging::log(&format!("GET {} -> {} bytes", api_url, resp.len()));
    crate::logging::log(&format!("GET response: {}", &resp[..resp.len().min(300)]));

    let api_resp = if resp.is_empty() || !resp.contains("tobeparsed") {
        crate::logging::log("GET had no tobeparsed, trying POST fallback");
        let body = serde_json::json!({"variables": {"showId": show_id, "translationType": mode_str, "episodeString": episode}, "query": episode_gql});
        let post_resp = client.post(format!("{}/api", allanime_api()))
            .headers(default_headers()).header("Content-Type", "application/json")
            .json(&body).send().await.map_err(|e| e.to_string())?
            .text().await.map_err(|e| e.to_string())?;
        crate::logging::log(&format!("POST response: {} bytes - {}", post_resp.len(), &post_resp[..post_resp.len().min(300)]));
        post_resp
    } else { resp };

    let sources: Vec<(String, String)> = if api_resp.contains("tobeparsed") {
        Regex::new(r#""tobeparsed":"([^"]*)""#).unwrap().captures(&api_resp)
            .map(|cap| decrypt_tobeparsed(&cap[1])).unwrap_or_default()
    } else {
        // Try multiple regex patterns for different response formats
        let mut s: Vec<(String, String)> = Regex::new(r#""sourceUrl":"--([^"]*)".*?"sourceName":"([^"]*)""#).unwrap()
            .captures_iter(&api_resp).map(|c| (c[2].to_string(), c[1].to_string())).collect();
        if s.is_empty() {
            // Alternate order: sourceName before sourceUrl
            s = Regex::new(r#""sourceName":"([^"]*)".*?"sourceUrl":"--([^"]*)""#).unwrap()
                .captures_iter(&api_resp).map(|c| (c[1].to_string(), c[2].to_string())).collect();
        }
        s
    };

    let futs: Vec<_> = sources.iter()
        .map(|(_, eid)| decode_provider_id(eid))
        .filter(|id| !id.is_empty())
        .map(|id| fetch_links(id)).collect();

    crate::logging::log(&format!("Sources found: {} | has_tobeparsed: {} | providers: {:?}", sources.len(), api_resp.contains("tobeparsed"), sources.iter().map(|(n,_)| n.as_str()).collect::<Vec<_>>()));
    for (name, eid) in &sources {
        crate::logging::log(&format!("  Provider '{}' encoded_id: {}... -> decoded: {}...", name, &eid[..eid.len().min(30)], &decode_provider_id(eid)[..decode_provider_id(eid).len().min(60)]));
    }

    let all_links: Vec<EpisodeLink> = futures::future::join_all(futs).await.into_iter().flatten().collect();
    crate::logging::log(&format!("Total links fetched: {}", all_links.len()));

    if !quality.is_empty() && quality != "best" {
        let filtered: Vec<EpisodeLink> = all_links.iter().filter(|l| l.quality.contains(&quality)).cloned().collect();
        if !filtered.is_empty() {
            cache_set(&cache.links, cache_key, filtered.clone());
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
            cache_set(&cache.links, cache_key, fallback.clone());
            return Ok(fallback);
        }
    }

    cache_set(&cache.links, cache_key, all_links.clone());
    Ok(all_links)
}

#[tauri::command]
pub async fn get_cache_stats(cache: State<'_, AppCache>) -> Result<CacheStats, String> {
    Ok(CacheStats {
        search_entries: cache.search.lock().map(|m| m.len()).unwrap_or(0),
        episode_entries: cache.episodes.lock().map(|m| m.len()).unwrap_or(0),
        link_entries: cache.links.lock().map(|m| m.len()).unwrap_or(0),
    })
}

#[tauri::command]
pub async fn clear_cache(cache: State<'_, AppCache>) -> Result<(), String> {
    cache.search.lock().map(|mut m| m.clear()).ok();
    cache.episodes.lock().map(|mut m| m.clear()).ok();
    cache.links.lock().map(|mut m| m.clear()).ok();
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
    let body = serde_json::json!({"variables": {"showId": show_id}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let name = Regex::new(r#""name":"([^"]*)"#).unwrap()
        .captures(&resp).map(|c| c[1].to_string()).unwrap_or_default();
    let description = Regex::new(r#""description":"((?:[^"\\]|\\.)*)""#).unwrap()
        .captures(&resp).map(|c| c[1].replace("\\n", "\n").replace("\\\"", "\"")).unwrap_or_default();
    let thumbnail = Regex::new(r#""thumbnail":"([^"]*)"#).unwrap()
        .captures(&resp).map(|c| c[1].to_string()).unwrap_or_default();
    let status = Regex::new(r#""status":"([^"]*)"#).unwrap()
        .captures(&resp).map(|c| c[1].to_string()).unwrap_or_else(|| "Unknown".to_string());
    let genres: Vec<String> = Regex::new(r#""genres":\[([^\]]*)\]"#).unwrap()
        .captures(&resp)
        .map(|c| c[1].split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let ep_count = Regex::new(r#""sub":\s*(\d+)"#).unwrap()
        .captures(&resp).and_then(|c| c[1].parse().ok()).unwrap_or(0);
    let rating = Regex::new(r#""score":\s*([0-9.]+)"#).unwrap()
        .captures(&resp).and_then(|c| c[1].parse().ok()).unwrap_or(0.0);

    Ok(AnimeDetails { id: show_id, name, description, thumbnail, genres, status, episode_count: ep_count, rating })
}

#[tauri::command]
pub async fn get_popular(mode: String, page: Option<u32>, cache: State<'_, AppCache>) -> Result<Vec<AnimeResult>, String> {
    let mode = if mode == "dub" { "dub" } else { "sub" };
    let pg = page.unwrap_or(1);
    let cache_key = format!("popular:{}:{}", mode, pg);
    if let Some(cached) = cache_valid(&cache.search, &cache_key) { return Ok(cached); }

    let gql = r#"query( $type: VaildShowObjectEnumType $page: Int $limit: Int $translationType: VaildTranslationTypeEnumType ) { queryPopular( type: $type page: $page limit: $limit translationType: $translationType ) { recommendations { anyCard { _id name thumbnail score malId availableEpisodes } } }}"#;
    let body = serde_json::json!({"variables": {"type": "anime", "page": pg, "limit": 40, "translationType": mode}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let mut results = Vec::new();
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
                    if !id.is_empty() { results.push(AnimeResult { id, name, episodes, thumbnail, rating, mal_id }); }
                }
            }
        }
    }
    cache_set(&cache.search, cache_key, results.clone());
    Ok(results)
}

#[tauri::command]
pub async fn get_recently_updated(mode: String, page: Option<u32>, cache: State<'_, AppCache>) -> Result<Vec<AnimeResult>, String> {
    let mode = if mode == "dub" { "dub" } else { "sub" };
    let pg = page.unwrap_or(1);
    let cache_key = format!("recent:{}:{}", mode, pg);
    if let Some(cached) = cache_valid(&cache.search, &cache_key) { return Ok(cached); }

    let gql = r#"query( $search: SearchInput $limit: Int $page: Int $translationType: VaildTranslationTypeEnumType $countryOrigin: VaildCountryOriginEnumType ) { shows( search: $search limit: $limit page: $page translationType: $translationType countryOrigin: $countryOrigin ) { edges { _id name thumbnail score malId availableEpisodes __typename } }}"#;
    let body = serde_json::json!({"variables": {"search": {"allowAdult": false, "allowUnknown": false, "query": ""}, "limit": 40, "page": pg, "translationType": mode, "countryOrigin": "ALL"}, "query": gql});

    let client = build_client();
    let resp = client.post(format!("{}/api", allanime_api()))
        .headers(default_headers()).header("Content-Type", "application/json")
        .json(&body).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
        if let Some(edges) = json.pointer("/data/shows/edges").and_then(|v| v.as_array()) {
            for edge in edges {
                let id = edge.get("_id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let name = edge.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let thumbnail = edge.get("thumbnail").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                let rating = edge.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let mal_id = edge.get("malId").and_then(|v| v.as_u64()).unwrap_or(0);
                let episodes = edge.pointer(&format!("/availableEpisodes/{}", mode)).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if episodes > 0 { results.push(AnimeResult { id, name, episodes, thumbnail, rating, mal_id }); }
            }
        }
    }
    cache_set(&cache.search, cache_key, results.clone());
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
    let mode_str = if mode == "dub" { "dub" } else { "sub" };
    let episode_gql = r#"query ($showId: String!, $translationType: VaildTranslationTypeEnumType!, $episodeString: String!) { episode( showId: $showId translationType: $translationType episodeString: $episodeString ) { episodeString sourceUrls }}"#;
    let query_hash = "d405d0edd690624b66baba3068e0edc3ac90f1597d898a1ec8db4e5c43c00fec";
    let vars = serde_json::json!({"showId": show_id, "translationType": mode_str, "episodeString": episode});
    let ext = serde_json::json!({"persistedQuery":{"version":1,"sha256Hash": query_hash}});

    let client = build_client();
    let vars_str = vars.to_string();
    let ext_str = ext.to_string();
    let encoded_vars = vars_str.replace('"', "%22").replace(':', "%3A").replace('{', "%7B").replace('}', "%7D").replace(',', "%2C");
    let encoded_ext = ext_str.replace('"', "%22").replace(':', "%3A").replace('{', "%7B").replace('}', "%7D").replace(',', "%2C").replace(' ', "%20");
    let api_url = format!("{}/api?variables={}&extensions={}", allanime_api(), encoded_vars, encoded_ext);

    let mut headers = default_headers();
    headers.insert("Origin", HeaderValue::from_static("https://youtu-chan.com"));
    headers.insert(REFERER, HeaderValue::from_static("https://youtu-chan.com"));

    let get_resp = client.get(&api_url).headers(headers).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;

    let mut info = DebugInfo {
        get_url: api_url.clone(),
        get_response_len: get_resp.len(),
        get_response_snippet: get_resp.chars().take(500).collect(),
        used_post_fallback: false,
        post_response_len: 0,
        post_response_snippet: String::new(),
        has_tobeparsed: false,
        sources_count: 0,
        sources: vec![],
        decoded_provider_ids: vec![],
        links_per_provider: vec![],
    };

    let api_resp = if get_resp.is_empty() || !get_resp.contains("tobeparsed") {
        info.used_post_fallback = true;
        let body = serde_json::json!({"variables": {"showId": show_id, "translationType": mode_str, "episodeString": episode}, "query": episode_gql});
        let post_resp = client.post(format!("{}/api", allanime_api()))
            .headers(default_headers()).header("Content-Type", "application/json")
            .json(&body).send().await.map_err(|e| e.to_string())?
            .text().await.map_err(|e| e.to_string())?;
        info.post_response_len = post_resp.len();
        info.post_response_snippet = post_resp.chars().take(500).collect();
        post_resp
    } else { get_resp.clone() };

    info.has_tobeparsed = api_resp.contains("tobeparsed");

    let sources: Vec<(String, String)> = if api_resp.contains("tobeparsed") {
        Regex::new(r#""tobeparsed":"([^"]*)""#).unwrap().captures(&api_resp)
            .map(|cap| decrypt_tobeparsed(&cap[1])).unwrap_or_default()
    } else {
        let mut s: Vec<(String, String)> = Regex::new(r#""sourceUrl":"--([^"]*)".*?"sourceName":"([^"]*)""#).unwrap()
            .captures_iter(&api_resp).map(|c| (c[2].to_string(), c[1].to_string())).collect();
        if s.is_empty() {
            s = Regex::new(r#""sourceName":"([^"]*)".*?"sourceUrl":"--([^"]*)""#).unwrap()
                .captures_iter(&api_resp).map(|c| (c[1].to_string(), c[2].to_string())).collect();
        }
        s
    };

    info.sources_count = sources.len();
    info.sources = sources.clone();

    for (name, eid) in &sources {
        let pid = decode_provider_id(eid);
        info.decoded_provider_ids.push(pid.clone());
        if !pid.is_empty() {
            let links = fetch_links(pid.clone()).await;
            info.links_per_provider.push((name.clone(), links.len()));
        }
    }

    Ok(info)
}

#[tauri::command]
pub async fn get_thumbnail_fallback(name: String, mal_id: Option<u64>) -> Result<String, String> {
    let client = build_client();
    // If we have MAL ID, use it directly (faster, more accurate)
    if let Some(id) = mal_id {
        if id > 0 {
            let url = format!("https://api.jikan.moe/v4/anime/{}", id);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(text) = resp.text().await {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(img) = json.pointer("/data/images/jpg/large_image_url").and_then(|v| v.as_str()) {
                            return Ok(img.to_string());
                        }
                    }
                }
            }
        }
    }
    // Fallback: search by name
    let url = format!("https://api.jikan.moe/v4/anime?q={}&limit=1", urlencoding::encode(&name));
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?
        .text().await.map_err(|e| e.to_string())?;
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
        if let Some(img) = json.pointer("/data/0/images/jpg/large_image_url").and_then(|v| v.as_str()) {
            return Ok(img.to_string());
        }
    }
    Err("No thumbnail found".to_string())
}
