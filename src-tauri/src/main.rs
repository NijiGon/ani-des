#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anilist;
mod aniskip;
mod api;
mod bookmarks;
mod consumet;
mod downloads;
mod history;
mod jikan;
pub mod logging;
mod proxy;
mod store;

use std::sync::Arc;
use tauri::Manager;
use anilist::{anilist_trending, anilist_popular, anilist_seasonal, anilist_search, anilist_detail, anilist_genres};
use aniskip::get_skip_times;
use api::{search_anime, get_episodes, get_episode_url, get_cache_stats, clear_cache, get_anime_details, get_popular, get_recently_updated, debug_episode_url, get_thumbnail_fallback, AppCache};
use bookmarks::{save_bookmark, get_bookmarks, delete_bookmark};
use downloads::{start_download, start_bulk_download, get_downloads, cancel_download, remove_download, open_download_folder, open_file, DownloadManager};
use history::{save_history, get_history, get_history_by_tag, set_history_tag, set_history_thumbnail, delete_history, get_all_tags};
use jikan::{jikan_search, jikan_top, jikan_genres, jikan_detail};
use logging::{get_log_path, read_logs};
use proxy::{proxy_url, local_url, ProxyState};

fn main() {
    let proxy_state = Arc::new(ProxyState::new());
    let proxy_for_task = proxy_state.clone();

    tauri::Builder::default()
        // A second launch would fail to bind the media proxy port; focus the existing window instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                window.set_focus().ok();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .manage(AppCache::new())
        .manage(DownloadManager::new())
        .manage(proxy_state)
        .setup(move |_app| {
            logging::prune_old_logs(14);
            tauri::async_runtime::spawn(async move {
                if let Err(e) = proxy::start_proxy(proxy_for_task).await {
                    logging::log(&format!("Media proxy failed to start: {}", e));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_anime, get_episodes, get_episode_url,
            get_cache_stats, clear_cache, get_anime_details,
            get_popular, get_recently_updated,
            debug_episode_url, get_thumbnail_fallback,
            anilist_trending, anilist_popular, anilist_seasonal,
            anilist_search, anilist_detail, anilist_genres,
            jikan_search, jikan_top, jikan_genres, jikan_detail,
            get_skip_times,

            save_history, get_history, get_history_by_tag, set_history_tag, set_history_thumbnail, delete_history, get_all_tags,
            save_bookmark, get_bookmarks, delete_bookmark,
            start_download, start_bulk_download, get_downloads,
            cancel_download, remove_download, open_download_folder, open_file,
            get_log_path, read_logs,
            proxy_url, local_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
