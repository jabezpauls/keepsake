//! Map tile proxy + on-disk cache.
//!
//! Routes every basemap tile fetch from MapLibre through a Tauri custom
//! URI scheme (`mvtile://{provider}/...`) so:
//!
//! 1. **Privacy.** The webview never opens a direct HTTP connection to
//!    OpenFreeMap / Esri / etc. Tauri's HTTP client does, with our own
//!    User-Agent and no third-party cookies / referrer leakage.
//!
//! 2. **Cache.** Hot tiles are served from disk on subsequent loads
//!    (no network at all). Cold tiles are fetched once and persisted.
//!    Total size is capped — when over, oldest mtimes are evicted.
//!
//! 3. **Single chokepoint.** A future PMTiles offline branch slots in
//!    here without touching the JS side.
//!
//! Supported providers (path layout after `mvtile://`):
//! - `openfreemap/styles/{name}`            — JSON style document.
//! - `openfreemap/tiles/{z}/{x}/{y}.pbf`    — vector tile.
//! - `openfreemap-fonts/{stack}/{range}.pbf` — SDF glyphs for labels.
//! - `openfreemap-sprite/{name}.{json|png}` — icon sprite.
//! - `esri/{z}/{y}/{x}`                     — raster satellite tile.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::http::{Request, Response, StatusCode};
use tauri::{Manager, UriSchemeContext};

const MAX_CACHE_BYTES: u64 = 500 * 1024 * 1024;
const CACHE_DIR_NAME: &str = "tile-cache";

/// Running total of bytes on disk in the cache directory. Updated as
/// inserts happen + evictions complete; initialised once on first
/// touch by walking the cache dir.
static CACHE_BYTES: Mutex<u64> = Mutex::new(0);
static CACHE_INITIALISED: Mutex<bool> = Mutex::new(false);

fn upstream_for(path: &str) -> Option<(String, &'static str)> {
    if let Some(name) = path.strip_prefix("openfreemap/styles/") {
        let name = name.trim_end_matches(".json");
        return Some((
            format!("https://tiles.openfreemap.org/styles/{name}"),
            "application/json",
        ));
    }
    // Vector tile path keeps the planet/{date}/ prefix end-to-end so we
    // don't have to hardcode dataset versions in this file — the JS
    // side just strips the upstream origin and we re-prepend it here.
    if let Some(suffix) = path.strip_prefix("openfreemap/tiles/") {
        return Some((
            format!("https://tiles.openfreemap.org/planet/{suffix}"),
            "application/x-protobuf",
        ));
    }
    if let Some(suffix) = path.strip_prefix("openfreemap-fonts/") {
        return Some((
            format!("https://tiles.openfreemap.org/fonts/{suffix}"),
            "application/x-protobuf",
        ));
    }
    if let Some(suffix) = path.strip_prefix("openfreemap-sprite/") {
        let mime = if suffix.ends_with(".png") {
            "image/png"
        } else {
            "application/json"
        };
        return Some((
            format!("https://tiles.openfreemap.org/sprites/ofm_f384/{suffix}"),
            mime,
        ));
    }
    if let Some(suffix) = path.strip_prefix("esri/") {
        return Some((
            format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{suffix}"
            ),
            "image/jpeg",
        ));
    }
    None
}

fn cache_path(root: &Path, key: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for seg in key.split('/') {
        p.push(seg);
    }
    p
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
    out
}

fn initialise_cache_size(root: &Path) {
    let mut init = CACHE_INITIALISED.lock().expect("cache init mutex");
    if *init {
        return;
    }
    let mut total: u64 = 0;
    for path in walk_files(root) {
        if let Ok(meta) = std::fs::metadata(&path) {
            total = total.saturating_add(meta.len());
        }
    }
    *CACHE_BYTES.lock().expect("cache size mutex") = total;
    *init = true;
}

/// LRU-ish eviction by oldest mtime when the running total goes over
/// the cap. Removes ~10% of cap so we don't churn on every insert.
fn evict_if_needed(root: &Path) {
    let mut total = CACHE_BYTES.lock().expect("cache size mutex");
    if *total <= MAX_CACHE_BYTES {
        return;
    }
    let target = MAX_CACHE_BYTES * 9 / 10;
    let mut sized: Vec<(PathBuf, std::time::SystemTime, u64)> = walk_files(root)
        .into_iter()
        .filter_map(|p| {
            let meta = std::fs::metadata(&p).ok()?;
            let mtime = meta.modified().ok()?;
            Some((p, mtime, meta.len()))
        })
        .collect();
    sized.sort_by_key(|(_, mtime, _)| *mtime);
    for (path, _, size) in sized {
        if *total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            *total = total.saturating_sub(size);
        }
    }
}

fn fetch_upstream(url: &str) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .header("User-Agent", "Keepsake/0.1 (offline-first photo library)")
        .call()
        .map_err(|e| format!("HTTP {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }
    let (_parts, mut body) = response.into_parts();
    let mut reader = body.as_reader();
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .map_err(|e| format!("read body {url}: {e}"))?;
    Ok(buf)
}

fn get_or_fetch(cache_root: &Path, key: &str, url: &str) -> Result<Vec<u8>, String> {
    initialise_cache_size(cache_root);
    let path = cache_path(cache_root, key);
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(bytes);
    }
    let bytes = fetch_upstream(url)?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, &bytes).is_ok() {
        if let Ok(mut total) = CACHE_BYTES.lock() {
            *total = total.saturating_add(bytes.len() as u64);
        }
        evict_if_needed(cache_root);
    }
    Ok(bytes)
}

pub fn handle_request<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let uri = request.uri();
    let host = uri.host().unwrap_or("");
    let path = uri.path().trim_start_matches('/');
    let key = if path.is_empty() {
        host.to_string()
    } else {
        format!("{host}/{path}")
    };

    let Some((url, mime)) = upstream_for(&key) else {
        return error_response(StatusCode::NOT_FOUND, "unknown tile provider");
    };

    let cache_root = match ctx.app_handle().path().app_cache_dir() {
        Ok(dir) => dir.join(CACHE_DIR_NAME),
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "no cache dir");
        }
    };

    match get_or_fetch(&cache_root, &key, &url) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime)
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "public, max-age=86400")
            .body(bytes)
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build resp")),
        Err(msg) => {
            tracing::warn!(target: "mv_app::tiles", key = %key, error = %msg, "tile fetch failed");
            error_response(StatusCode::BAD_GATEWAY, &msg)
        }
    }
}

fn error_response(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .header("Access-Control-Allow-Origin", "*")
        .body(msg.as_bytes().to_vec())
        .expect("build error response")
}
