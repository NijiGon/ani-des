use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn log_dir() -> PathBuf {
    let dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("ani-des").join("logs");
    fs::create_dir_all(&dir).ok();
    dir
}

fn today() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = now / 86400;
    format!("{}", days)
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let h = (now % 86400) / 3600;
    let m = (now % 3600) / 60;
    let s = now % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

pub fn log(msg: &str) {
    let path = log_dir().join(format!("{}.log", today()));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        writeln!(f, "[{}] {}", timestamp(), msg).ok();
    }
}

#[tauri::command]
pub async fn get_log_path() -> Result<String, String> {
    let path = log_dir().join(format!("{}.log", today()));
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn read_logs() -> Result<String, String> {
    let path = log_dir().join(format!("{}.log", today()));
    fs::read_to_string(&path).map_err(|e| e.to_string())
}
