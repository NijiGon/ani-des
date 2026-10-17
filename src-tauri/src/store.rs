use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// One lock for all JSON stores: read-modify-write calls must not interleave.
static LOCK: Mutex<()> = Mutex::new(());

pub fn data_dir() -> PathBuf {
    let dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("ani-des");
    fs::create_dir_all(&dir).ok();
    dir
}

fn read<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(text) = fs::read_to_string(path) else { return T::default() };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            // Keep the unreadable file for recovery instead of overwriting it with defaults.
            fs::rename(path, path.with_extension("corrupt")).ok();
            T::default()
        }
    }
}

fn write<T: Serialize>(path: &Path, value: &T) {
    let Ok(json) = serde_json::to_string_pretty(value) else { return };
    // Write to a temp file first so a crash mid-write cannot truncate the store.
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, json).is_ok() {
        fs::rename(&tmp, path).ok();
    }
}

fn locked<R>(f: impl FnOnce() -> R) -> R {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

pub fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    locked(|| read(&path))
}

pub fn save<T: Serialize>(name: &str, value: &T) {
    let path = data_dir().join(name);
    locked(|| write(&path, value));
}

/// Read, mutate and write one JSON file as a single locked step.
pub fn update<T, R>(name: &str, f: impl FnOnce(&mut T) -> R) -> R
where
    T: DeserializeOwned + Serialize + Default,
{
    update_at(&data_dir().join(name), f)
}

pub fn update_at<T, R>(path: &Path, f: impl FnOnce(&mut T) -> R) -> R
where
    T: DeserializeOwned + Serialize + Default,
{
    locked(|| {
        let mut value: T = read(path);
        let out = f(&mut value);
        write(path, &value);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_roundtrip_and_corrupt_recovery() {
        let dir = std::env::temp_dir().join(format!("anides-store-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("list.json");

        update_at::<Vec<u32>, _>(&path, |v| v.push(7));
        update_at::<Vec<u32>, _>(&path, |v| v.push(9));
        assert_eq!(read::<Vec<u32>>(&path), vec![7, 9]);

        fs::write(&path, "{not json").unwrap();
        assert!(read::<Vec<u32>>(&path).is_empty());
        assert!(path.with_extension("corrupt").exists());

        fs::remove_dir_all(&dir).ok();
    }
}
