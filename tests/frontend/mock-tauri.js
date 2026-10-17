// Stands in for the Tauri IPC bridge. Canned data includes a hostile title to check escaping.
// Timer-based animation frames: background browser panes pause requestAnimationFrame, which stalls GSAP's transitions.
window.requestAnimationFrame = cb => setTimeout(() => cb(performance.now()), 16);
window.__calls = [];
window.__violations = [];
document.addEventListener("securitypolicyviolation", e => window.__violations.push(`${e.violatedDirective} ${e.blockedURI}`));

const HOSTILE = '<img src=x onerror="window.__pwned=1">Hostile';
const SHOW = { id: "show1", name: "Show One", thumbnail: "https://img.example/s1.jpg", episodes: 3, rating: 8, mal_id: 0 };

function respond(cmd, args) {
  switch (cmd) {
    case "get_cache_stats": return { search_entries: 1, episode_entries: 0, link_entries: 0 };
    case "get_recently_updated":
      return args.page === 1 ? [{ id: "hostile1", title: HOSTILE, cover: "https://img.example/a.jpg", episodes: 12, score: 8.1, mal_id: 1 }] : [];
    case "search_anime": return [SHOW];
    case "get_episodes": return ["1", "2", "3"];
    case "get_anime_details": return { id: "show1", name: "Show One", description: "Desc <b>x</b>", thumbnail: "", genres: ["Action"], status: "FINISHED", episode_count: 3, rating: 8 };
    case "get_episode_url": return [{ quality: "1080p", url: "https://cdn.example/v.mp4", referrer: null }, { quality: "720p", url: "https://cdn.example/m.m3u8", referrer: null }];
    case "proxy_url": return "http://127.0.0.1:18293/video?k=test";
    case "get_history":
    case "get_history_by_tag":
    case "get_bookmarks":
    case "get_downloads": return [];
    case "get_thumbnail_fallback": throw new Error("no thumbnail");
    case "jikan_detail": throw new Error("offline");
    case "get_skip_times": return [];
    default: return null;
  }
}

window.__TAURI__ = {
  core: {
    invoke: async (cmd, args = {}) => {
      window.__calls.push(cmd);
      return respond(cmd, args);
    },
  },
};
