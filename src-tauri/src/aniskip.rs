use serde::{Deserialize, Serialize};

const ANISKIP_BASE: &str = "https://api.aniskip.com/v2";

#[derive(Serialize, Deserialize, Clone)]
pub struct SkipTime {
    pub skip_type: String, // "op", "ed", "recap", "mixed-op", "mixed-ed"
    pub start: f64,
    pub end: f64,
}

#[tauri::command]
pub async fn get_skip_times(mal_id: u64, episode: u32, ep_length: Option<f64>) -> Result<Vec<SkipTime>, String> {
    let length = ep_length.unwrap_or(0.0) as u64;
    let url = format!(
        "{}/skip-times/{}/{}?types[]=op&types[]=ed&types[]=recap&types[]=mixed-op&types[]=mixed-ed{}",
        ANISKIP_BASE, mal_id, episode,
        if length > 0 { format!("&episodeLength={}", length) } else { String::new() }
    );

    let resp = crate::api::http().get(&url).timeout(std::time::Duration::from_secs(5)).send().await.map_err(|e| e.to_string())?
        .json::<serde_json::Value>().await.map_err(|e| e.to_string())?;

    let mut times = Vec::new();
    if let Some(results) = resp.get("results").and_then(|v| v.as_array()) {
        for r in results {
            let skip_type = r.get("skipType").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let start = r.pointer("/interval/startTime").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let end = r.pointer("/interval/endTime").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if end > start { times.push(SkipTime { skip_type, start, end }); }
        }
    }
    Ok(times)
}
