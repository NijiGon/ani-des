use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const JIKAN_BASE: &str = "https://api.jikan.moe/v4";
// Jikan allows about 3 requests/second; keep a safe gap between request starts.
const MIN_GAP: Duration = Duration::from_millis(400);

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap()
}

fn gate() -> &'static tokio::sync::Mutex<Option<Instant>> {
    static GATE: OnceLock<tokio::sync::Mutex<Option<Instant>>> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// GET a Jikan URL as JSON. Requests are serialized through one gate so bursts
/// (e.g. many thumbnail fallbacks at once) stay under the rate limit.
pub async fn jikan_get(url: &str) -> Result<serde_json::Value, String> {
    for attempt in 0..2 {
        {
            let mut last = gate().lock().await;
            if let Some(t) = *last {
                if let Some(wait) = MIN_GAP.checked_sub(t.elapsed()) {
                    tokio::time::sleep(wait).await;
                }
            }
            *last = Some(Instant::now());
        }
        let resp = client().get(url).send().await.map_err(|e| e.to_string())?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt == 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        return resp.json::<serde_json::Value>().await.map_err(|e| e.to_string());
    }
    Err("Jikan rate limited".to_string())
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct JikanAnime {
    pub mal_id: u64,
    pub title: String,
    pub title_english: String,
    pub title_japanese: String,
    pub image: String,
    pub synopsis: String,
    pub genres: Vec<String>,
    pub score: f64,
    pub episodes: u32,
    pub status: String,
    pub season: String,
    pub year: u32,
    pub r#type: String,
    pub rating: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct JikanGenre {
    pub mal_id: u64,
    pub name: String,
}

fn parse_anime(data: &serde_json::Value) -> Option<JikanAnime> {
    let mal_id = data.get("mal_id")?.as_u64()?;
    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let title_english = data.pointer("/title_english").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let title_japanese = data.pointer("/title_japanese").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let image = data.pointer("/images/jpg/large_image_url").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let synopsis = data.get("synopsis").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let genres: Vec<String> = data.get("genres").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|g| g.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let score = data.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let episodes = data.get("episodes").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let status = data.get("status").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let season = data.get("season").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let year = data.get("year").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let r#type = data.get("type").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let rating = data.get("rating").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    Some(JikanAnime { mal_id, title, title_english, title_japanese, image, synopsis, genres, score, episodes, status, season, year, r#type, rating })
}

fn parse_list(resp: &serde_json::Value) -> Vec<JikanAnime> {
    resp.get("data").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(parse_anime).collect())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn jikan_search(
    query: Option<String>,
    genre: Option<String>,
    year: Option<u32>,
    season: Option<String>,
    r#type: Option<String>,
    status: Option<String>,
    order_by: Option<String>,
    page: Option<u32>,
) -> Result<Vec<JikanAnime>, String> {
    let mut params = vec![format!("page={}", page.unwrap_or(1)), "limit=25".to_string(), "sfw=true".to_string()];
    if let Some(q) = query { if !q.is_empty() { params.push(format!("q={}", urlencoding::encode(&q))); } }
    if let Some(g) = genre { params.push(format!("genres={}", g)); } // genre ID
    if let Some(y) = year { params.push(format!("start_date={}-01-01", y)); params.push(format!("end_date={}-12-31", y)); }
    if let Some(t) = r#type { params.push(format!("type={}", t)); }
    if let Some(s) = status { params.push(format!("status={}", s)); }
    if let Some(o) = order_by { params.push(format!("order_by={}", o)); params.push("sort=desc".to_string()); }

    let url = if let Some(s) = season {
        let y = year.unwrap_or_else(|| crate::logging::today_ymd().0 as u32);
        format!("{}/seasons/{}/{}", JIKAN_BASE, y, s.to_lowercase())
    } else {
        format!("{}/anime?{}", JIKAN_BASE, params.join("&"))
    };

    Ok(parse_list(&jikan_get(&url).await?))
}

#[tauri::command]
pub async fn jikan_top(filter: Option<String>, page: Option<u32>) -> Result<Vec<JikanAnime>, String> {
    let f = filter.unwrap_or_else(|| "airing".to_string());
    let url = format!("{}/top/anime?filter={}&page={}&limit=25", JIKAN_BASE, f, page.unwrap_or(1));
    Ok(parse_list(&jikan_get(&url).await?))
}

#[tauri::command]
pub async fn jikan_genres() -> Result<Vec<JikanGenre>, String> {
    let resp = jikan_get(&format!("{}/genres/anime", JIKAN_BASE)).await?;
    let genres = resp.get("data").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|g| {
            let id = g.get("mal_id")?.as_u64()?;
            let name = g.get("name")?.as_str()?.to_string();
            Some(JikanGenre { mal_id: id, name })
        }).collect()).unwrap_or_default();
    Ok(genres)
}

#[tauri::command]
pub async fn jikan_detail(mal_id: u64) -> Result<JikanAnime, String> {
    let resp = jikan_get(&format!("{}/anime/{}/full", JIKAN_BASE, mal_id)).await?;
    resp.get("data").and_then(parse_anime).ok_or_else(|| "Not found".to_string())
}
