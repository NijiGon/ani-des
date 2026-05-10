use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

const ANILIST_API: &str = "https://graphql.anilist.co";

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap()
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AniListMedia {
    pub id: u64,
    pub title: String,
    pub title_romaji: String,
    pub cover: String,
    pub banner: String,
    pub description: String,
    pub genres: Vec<String>,
    pub score: f64,
    pub episodes: u32,
    pub status: String,
    pub season: String,
    pub year: u32,
    pub format: String,
    pub studios: Vec<String>,
}

fn parse_media(m: &serde_json::Value) -> Option<AniListMedia> {
    let id = m.get("id")?.as_u64()?;
    let title = m.pointer("/title/english").and_then(|v| v.as_str()).or_else(|| m.pointer("/title/romaji").and_then(|v| v.as_str())).unwrap_or_default().to_string();
    let title_romaji = m.pointer("/title/romaji").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let cover = m.pointer("/coverImage/large").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let banner = m.get("bannerImage").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let description = m.get("description").and_then(|v| v.as_str()).unwrap_or_default().replace("<br>", "\n").replace("<i>", "").replace("</i>", "").replace("<b>", "").replace("</b>", "");
    let genres: Vec<String> = m.get("genres").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|g| g.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let score = m.get("averageScore").and_then(|v| v.as_f64()).unwrap_or(0.0) / 10.0;
    let episodes = m.get("episodes").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let status = m.get("status").and_then(|v| v.as_str()).unwrap_or("UNKNOWN").to_string();
    let season = m.get("season").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let year = m.get("seasonYear").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let format = m.get("format").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let studios: Vec<String> = m.pointer("/studios/nodes").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect()).unwrap_or_default();
    Some(AniListMedia { id, title, title_romaji, cover, banner, description, genres, score, episodes, status, season, year, format, studios })
}

async fn query(gql: &str, vars: serde_json::Value) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({"query": gql, "variables": vars});
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    client().post(ANILIST_API).headers(headers).json(&body).send().await.map_err(|e| e.to_string())?
        .json::<serde_json::Value>().await.map_err(|e| e.to_string())
}

const MEDIA_FIELDS: &str = "id title{romaji english} coverImage{large} bannerImage description genres averageScore episodes status season seasonYear format studios{nodes{name}}";

#[tauri::command]
pub async fn anilist_trending() -> Result<Vec<AniListMedia>, String> {
    let gql = format!("query {{ Page(page:1,perPage:20) {{ media(type:ANIME,sort:TRENDING_DESC) {{ {} }} }} }}", MEDIA_FIELDS);
    let resp = query(&gql, serde_json::json!({})).await?;
    let media = resp.pointer("/data/Page/media").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(parse_media).collect()).unwrap_or_default();
    Ok(media)
}

#[tauri::command]
pub async fn anilist_popular() -> Result<Vec<AniListMedia>, String> {
    let gql = format!("query {{ Page(page:1,perPage:20) {{ media(type:ANIME,sort:POPULARITY_DESC) {{ {} }} }} }}", MEDIA_FIELDS);
    let resp = query(&gql, serde_json::json!({})).await?;
    Ok(resp.pointer("/data/Page/media").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(parse_media).collect()).unwrap_or_default())
}

#[tauri::command]
pub async fn anilist_seasonal(season: String, year: u32) -> Result<Vec<AniListMedia>, String> {
    let gql = format!("query($s:MediaSeason,$y:Int) {{ Page(page:1,perPage:30) {{ media(type:ANIME,season:$s,seasonYear:$y,sort:POPULARITY_DESC) {{ {} }} }} }}", MEDIA_FIELDS);
    let resp = query(&gql, serde_json::json!({"s": season.to_uppercase(), "y": year})).await?;
    Ok(resp.pointer("/data/Page/media").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(parse_media).collect()).unwrap_or_default())
}

#[tauri::command]
pub async fn anilist_search(query_str: String, genre: Option<String>, year: Option<u32>, season: Option<String>, sort: Option<String>, format: Option<String>) -> Result<Vec<AniListMedia>, String> {
    let mut filters = vec!["type:ANIME".to_string()];
    let mut vars = serde_json::Map::new();

    if !query_str.is_empty() { filters.push("search:$q".to_string()); vars.insert("q".to_string(), serde_json::json!(query_str)); }
    if let Some(g) = genre { filters.push("genre:$g".to_string()); vars.insert("g".to_string(), serde_json::json!(g)); }
    if let Some(y) = year { filters.push("seasonYear:$y".to_string()); vars.insert("y".to_string(), serde_json::json!(y)); }
    if let Some(s) = season { filters.push("season:$s".to_string()); vars.insert("s".to_string(), serde_json::json!(s.to_uppercase())); }
    if let Some(f) = format { filters.push("format:$f".to_string()); vars.insert("f".to_string(), serde_json::json!(f)); }

    let sort_val = sort.unwrap_or_else(|| "POPULARITY_DESC".to_string());
    filters.push(format!("sort:{}", sort_val));

    let var_defs: Vec<String> = vars.keys().map(|k| match k.as_str() {
        "q" => "$q:String".to_string(), "g" => "$g:String".to_string(),
        "y" => "$y:Int".to_string(), "s" => "$s:MediaSeason".to_string(),
        "f" => "$f:MediaFormat".to_string(), _ => String::new(),
    }).filter(|s| !s.is_empty()).collect();

    let var_def_str = if var_defs.is_empty() { String::new() } else { format!("({})", var_defs.join(",")) };
    let gql = format!("query{} {{ Page(page:1,perPage:30) {{ media({}) {{ {} }} }} }}", var_def_str, filters.join(","), MEDIA_FIELDS);

    let resp = query(&gql, serde_json::Value::Object(vars)).await?;
    Ok(resp.pointer("/data/Page/media").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(parse_media).collect()).unwrap_or_default())
}

#[tauri::command]
pub async fn anilist_detail(id: u64) -> Result<AniListMedia, String> {
    let gql = format!("query($id:Int) {{ Media(id:$id,type:ANIME) {{ {} }} }}", MEDIA_FIELDS);
    let resp = query(&gql, serde_json::json!({"id": id})).await?;
    resp.pointer("/data/Media").and_then(parse_media).ok_or_else(|| "Not found".to_string())
}

#[tauri::command]
pub async fn anilist_genres() -> Result<Vec<String>, String> {
    let gql = "query { GenreCollection }";
    let resp = query(gql, serde_json::json!({})).await?;
    Ok(resp.pointer("/data/GenreCollection").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|g| g.as_str().map(|s| s.to_string())).collect()).unwrap_or_default())
}
