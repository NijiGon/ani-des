use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn log_dir() -> PathBuf {
    let dir = crate::store::data_dir().join("logs");
    fs::create_dir_all(&dir).ok();
    dir
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's civil_from_days.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Today's (year, month, day) in UTC.
pub fn today_ymd() -> (i64, i64, i64) {
    civil_from_days((now_secs() / 86400) as i64)
}

fn log_path() -> PathBuf {
    let (y, m, d) = today_ymd();
    log_dir().join(format!("{:04}-{:02}-{:02}.log", y, m, d))
}

fn timestamp() -> String {
    let now = now_secs();
    format!("{:02}:{:02}:{:02}", (now % 86400) / 3600, (now % 3600) / 60, now % 60)
}

pub fn log(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_path()) {
        writeln!(f, "[{}] {}", timestamp(), msg).ok();
    }
}

#[tauri::command]
pub async fn get_log_path() -> Result<String, String> {
    Ok(log_path().to_string_lossy().to_string())
}

#[tauri::command]
pub async fn read_logs() -> Result<String, String> {
    fs::read_to_string(log_path()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
