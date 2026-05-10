# AniDes

A desktop anime streaming app built with Tauri + Rust.

![AniDes](https://img.shields.io/badge/version-0.2.0-00d4aa)

## Features

- **Stream anime** with built-in video player (HLS + MP4)
- **Custom video player** with YouTube-like keyboard shortcuts
- **AniList integration** for trending, popular, and genre-filtered browsing
- **AllAnime** as streaming source with automatic provider resolution
- **Library** with tags: Watching, Plan to Watch, On Break, Finished, Dropped
- **Download manager** — bulk/selective downloads with progress tracking
- **Watch history** with resume from last position
- **Search** with autocomplete, genre filter, mode (sub/dub), and origin filter
- **Bookmarked searches** — save filter combos for quick access
- **In-memory caching** for fast repeated lookups
- **GSAP animations** and page transitions
- **Breadcrumb navigation**

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

- **Frontend**: HTML/CSS/JS (single file), HLS.js, GSAP, Lucide Icons
- **Backend**: Rust + Tauri v2
- **APIs**: AllAnime (streaming), AniList (metadata)
- **Video proxy**: Local TCP proxy for referrer injection and local file serving

## Build

Requires [Rust](https://rustup.rs/) and the Tauri prerequisites.

```bash
cd src-tauri
cargo build --release
```

The binary will be at `src-tauri/target/release/ani-des.exe`.

## Data Storage

| Data | Location |
|------|----------|
| Watch history | `%LOCALAPPDATA%\ani-des\history.json` |
| Bookmarks | `%LOCALAPPDATA%\ani-des\bookmarks.json` |
| Downloads DB | `%LOCALAPPDATA%\ani-des\downloads.json` |
| Downloaded videos | `%USERPROFILE%\Videos\ani-des\` |
| Logs | `%LOCALAPPDATA%\ani-des\logs\` |

## License

GPL-3.0
