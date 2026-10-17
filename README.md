# AniDes

A desktop anime streaming app built with Tauri + Rust.

![AniDes](https://img.shields.io/badge/version-0.3.0-00d4aa)

## Features

- **Stream anime** with a built-in player (HLS and MP4)
- **Custom video player** with YouTube-like keyboard shortcuts
- **AniList integration** for trending, popular, and genre-filtered browsing
- **AllAnime** as the main streaming source, with **Gogoanime** as a fallback
- **Library** with tags: Watching, Plan to Watch, On Break, Finished, Dropped
- **Download manager**: bulk or selective downloads with progress and cancel
  - Direct MP4 sources are preferred; HLS streams are saved as `.ts`
  - AES-128 encrypted HLS is decrypted during download
  - Fragmented MP4 (fMP4) HLS and other DRM schemes are not supported
- **Offline playback** of downloaded episodes, including HLS downloads
- **Watch history** with resume from last position
- **Search** with autocomplete, mode (sub/dub), and origin filter
- **Bookmarked searches** to save filter combinations
- **Skip intro/outro** using AniSkip timings
- **Breadcrumb navigation** and back/forward history

## Keyboard Shortcuts (Player)

| Key | Action |
|-----|--------|
| Space / K | Play/Pause |
| J / ← | Skip back 5s |
| L / → | Skip forward 5s |
| ↑ / ↓ | Volume up/down |
| F | Fullscreen |
| M | Mute |
| Shift+N | Next episode |
| Shift+P | Previous episode |
| > | Cycle playback speed |

## Tech Stack

- **Frontend**: HTML, CSS and JavaScript (`src/index.html`, `src/app.js`), HLS.js, GSAP, Lucide icons
- **Backend**: Rust + Tauri v2
- **APIs**: AllAnime (streaming), AniList and Jikan (metadata), AniSkip (skip times), Gogoanime (fallback)
- **Media proxy**: a local HTTP server on `127.0.0.1:18293`. It relays remote streams with the correct referrer, serves downloaded files, and serves saved HLS playlists. Every request needs a random token generated at launch.

## Security

- The window's Content-Security-Policy allows scripts only from the app itself and three CDNs (hls.js, GSAP, Lucide).
- API data is HTML-escaped before it reaches the page, and event handlers are attached by delegation, not inline.
- The media proxy only reads files inside the download folder.
- Only one instance runs at a time; a second launch focuses the first window.

## Build

Requires [Rust](https://rustup.rs/) and the Tauri prerequisites.

```bash
cd src-tauri
cargo build --release
```

The binary will be at `src-tauri/target/release/ani-des.exe`.

## Tests

Rust unit and integration tests:

```bash
cd src-tauri
cargo test
```

Frontend smoke tests run the real `src/index.html` in a browser with a mocked Tauri bridge and the app's CSP:

```bash
node tests/frontend/serve.js
```

Then open `http://localhost:8080/tests/frontend/`. The page shows PASS/FAIL for each check.

CI (`.github/workflows/ci.yml`) runs the Rust tests and syntax checks on every push.

## Data Storage

| Data | Location |
|------|----------|
| Watch history | `%LOCALAPPDATA%\ani-des\history.json` |
| Bookmarks | `%LOCALAPPDATA%\ani-des\bookmarks.json` |
| Downloads list | `%LOCALAPPDATA%\ani-des\downloads.json` |
| Search and link cache (5 min) | `%LOCALAPPDATA%\ani-des\cache\` |
| Downloaded videos | `%USERPROFILE%\Videos\ani-des\` |
| Logs (kept 14 days) | `%LOCALAPPDATA%\ani-des\logs\` |

Store files are written atomically. A file that cannot be read is renamed with a `.corrupt` extension instead of being overwritten.

## License

GPL-3.0
