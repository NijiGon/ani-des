#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anilist;
mod api;
mod bookmarks;
mod consumet;
mod downloads;
mod history;
pub mod logging;
mod proxy;

use std::sync::Arc;
use anilist::{anilist_trending, anilist_popular, anilist_seasonal, anilist_search, anilist_detail, anilist_genres};
use api::{search_anime, get_episodes, get_episode_url, get_cache_stats, clear_cache, get_anime_details, get_popular, get_recently_updated, debug_episode_url, AppCache};
use bookmarks::{save_bookmark, get_bookmarks, delete_bookmark};
use consumet::consumet_get_sources;
use downloads::{start_download, start_bulk_download, get_downloads, cancel_download, remove_download, open_download_folder, open_file, DownloadManager};
use history::{save_history, get_history, get_history_by_tag, set_history_tag, set_history_thumbnail, delete_history, get_all_tags};
use logging::{get_log_path, read_logs};
use proxy::{set_proxy_url, ProxyState};

fn main() {
    let proxy_state = Arc::new(ProxyState::new());
    let proxy_state_clone = proxy_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppCache::new())
        .manage(DownloadManager::new())
        .manage(proxy_state)
        .setup(move |_app| {
            tauri::async_runtime::spawn(async move {
                proxy::start_proxy(proxy_state_clone).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_anime, get_episodes, get_episode_url,
            get_cache_stats, clear_cache, get_anime_details,
            get_popular, get_recently_updated,
            debug_episode_url,
            anilist_trending, anilist_popular, anilist_seasonal,
            anilist_search, anilist_detail, anilist_genres,
            consumet_get_sources,
            save_history, get_history, get_history_by_tag, set_history_tag, set_history_thumbnail, delete_history, get_all_tags,
            save_bookmark, get_bookmarks, delete_bookmark,
            start_download, start_bulk_download, get_downloads,
            cancel_download, remove_download, open_download_folder, open_file,
            get_log_path, read_logs,
            set_proxy_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
