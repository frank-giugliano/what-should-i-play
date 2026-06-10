use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// ── Cache ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Cache {
    conn: Arc<Mutex<Connection>>,
    ttl: u64,
    library_ttl: u64,
    cache_images: bool,
}

impl Cache {
    fn new(ttl: u64, library_ttl: u64, cache_images: bool) -> Self {
        let conn = Connection::open(binary_dir().join("cache.db")).expect("Failed to open cache.db");
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS rawg_cache (
                game_name   TEXT PRIMARY KEY,
                data        TEXT NOT NULL,
                cached_at   INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS reviews_cache (
                appid       TEXT PRIMARY KEY,
                data        TEXT NOT NULL,
                cached_at   INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS library_cache (
                steam_id    TEXT PRIMARY KEY,
                data        TEXT NOT NULL,
                cached_at   INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS summary_cache (
                appid       TEXT PRIMARY KEY,
                summary     TEXT NOT NULL,
                cached_at   INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playtime_cache (
                appid       TEXT PRIMARY KEY,
                data        TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS hltb_cache (
                appid       TEXT PRIMARY KEY,
                data        TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS game_images (
                appid       TEXT PRIMARY KEY,
                header_url  TEXT NOT NULL,
                cached_at   INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS snoozed (
                appid       TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                wake_date   INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS slug_overrides (
                game_name   TEXT PRIMARY KEY,
                slug        TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS trailer_overrides (
                appid       TEXT PRIMARY KEY,
                youtube_id  TEXT NOT NULL
            );
        ").expect("Failed to create cache tables");
        if cache_images {
            std::fs::create_dir_all(binary_dir().join("image_cache")).expect("Failed to create image_cache dir");
        }
        Cache { conn: Arc::new(Mutex::new(conn)), ttl, library_ttl, cache_images }
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }

    fn get_rawg(&self, name: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;
        let result: rusqlite::Result<(String, u64)> = conn.query_row(
            "SELECT data, cached_at FROM rawg_cache WHERE game_name = ?1",
            params![name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((data, cached_at)) if Self::now() - cached_at < self.ttl => {
                serde_json::from_str(&data).ok()
            }
            _ => None,
        }
    }

    fn set_rawg(&self, name: &str, data: &Value) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO rawg_cache (game_name, data, cached_at) VALUES (?1, ?2, ?3)",
                params![name, data.to_string(), Self::now()],
            );
        }
    }

    fn get_reviews(&self, appid: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;
        let result: rusqlite::Result<(String, u64)> = conn.query_row(
            "SELECT data, cached_at FROM reviews_cache WHERE appid = ?1",
            params![appid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((data, cached_at)) if Self::now() - cached_at < self.ttl => {
                serde_json::from_str(&data).ok()
            }
            _ => None,
        }
    }

    fn set_reviews(&self, appid: &str, data: &Value) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO reviews_cache (appid, data, cached_at) VALUES (?1, ?2, ?3)",
                params![appid, data.to_string(), Self::now()],
            );
        }
    }

    fn get_library(&self, steam_id: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;
        let result: rusqlite::Result<(String, u64)> = conn.query_row(
            "SELECT data, cached_at FROM library_cache WHERE steam_id = ?1",
            params![steam_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((data, cached_at)) if Self::now() - cached_at < self.library_ttl => {
                serde_json::from_str(&data).ok()
            }
            _ => None,
        }
    }

    fn set_library(&self, steam_id: &str, data: &Value) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO library_cache (steam_id, data, cached_at) VALUES (?1, ?2, ?3)",
                params![steam_id, data.to_string(), Self::now()],
            );
        }
    }

    fn get_game_image(&self, appid: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        let ttl = self.ttl * 4; // 30 days roughly
        let result: rusqlite::Result<(String, u64)> = conn.query_row(
            "SELECT header_url, cached_at FROM game_images WHERE appid = ?1",
            params![appid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((url, cached_at)) if Self::now() - cached_at < ttl => Some(url),
            _ => None,
        }
    }

    fn set_game_image(&self, appid: &str, url: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO game_images (appid, header_url, cached_at) VALUES (?1, ?2, ?3)",
                params![appid, url, Self::now()],
            );
        }
    }

    fn get_hltb(&self, appid: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;
        let result: rusqlite::Result<String> = conn.query_row(
            "SELECT data FROM hltb_cache WHERE appid = ?1",
            params![appid],
            |row| row.get(0),
        );
        result.ok().and_then(|d| serde_json::from_str(&d).ok())
    }

    fn set_hltb(&self, appid: &str, data: &Value) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO hltb_cache (appid, data) VALUES (?1, ?2)",
                params![appid, data.to_string()],
            );
        }
    }

    fn get_playtime(&self, appid: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;
        let result: rusqlite::Result<String> = conn.query_row(
            "SELECT data FROM playtime_cache WHERE appid = ?1",
            params![appid],
            |row| row.get(0),
        );
        result.ok().and_then(|d| serde_json::from_str(&d).ok())
    }

    fn set_playtime(&self, appid: &str, data: &Value) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO playtime_cache (appid, data) VALUES (?1, ?2)",
                params![appid, data.to_string()],
            );
        }
    }

    fn get_summary(&self, appid: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        let result: rusqlite::Result<(String, u64)> = conn.query_row(
            "SELECT summary, cached_at FROM summary_cache WHERE appid = ?1",
            params![appid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok((summary, cached_at)) if Self::now() - cached_at < summary_cache_ttl() => Some(summary),
            _ => None,
        }
    }

    fn set_summary(&self, appid: &str, summary: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO summary_cache (appid, summary, cached_at) VALUES (?1, ?2, ?3)",
                params![appid, summary, Self::now()],
            );
        }
    }

    fn image_cache_path(url: &str) -> String {
        let slug = url.rsplit('/').next().unwrap_or("img")
            .chars().filter(|c| c.is_alphanumeric() || *c == '.').collect::<String>();
        let hash = url.len();
        binary_dir().join(format!("image_cache/{}_{}", hash, slug))
            .to_string_lossy().to_string()
    }

    fn get_image(&self, url: &str) -> Option<String> {
        if !self.cache_images { return None; }
        let path = Self::image_cache_path(url);
        if std::path::Path::new(&path).exists() {
            Some(format!("/img_cache/{}_{}", url.len(),
                url.rsplit('/').next().unwrap_or("img")
                    .chars().filter(|c| c.is_alphanumeric() || *c == '.').collect::<String>()))
        } else {
            None
        }
    }

    fn get_trailer_override(&self, appid: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT youtube_id FROM trailer_overrides WHERE appid = ?1",
            params![appid],
            |row| row.get(0),
        ).ok()
    }

    fn set_trailer_override(&self, appid: &str, youtube_id: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO trailer_overrides (appid, youtube_id) VALUES (?1, ?2)",
                params![appid, youtube_id],
            );
        }
    }

    fn get_slug_override(&self, name: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT slug FROM slug_overrides WHERE game_name = ?1",
            params![name],
            |row| row.get(0),
        ).ok()
    }

    fn set_slug_override(&self, name: &str, slug: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO slug_overrides (game_name, slug) VALUES (?1, ?2)",
                params![name, slug],
            );
        }
    }

    fn clear_table(&self, table: &str) -> usize {
        if let Ok(conn) = self.conn.lock() {
            conn.execute(&format!("DELETE FROM {}", table), [])
                .unwrap_or(0)
        } else { 0 }
    }

    fn table_count(&self, table: &str) -> usize {
        if let Ok(conn) = self.conn.lock() {
            conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |row| row.get(0))
                .unwrap_or(0)
        } else { 0 }
    }

    fn purge_stale_images(&self) {
        if !self.cache_images { return; }
        let dir = binary_dir().join("image_cache");
        if !dir.exists() { return; }
        let cutoff = Self::now().saturating_sub(self.ttl);
        let mut purged = 0u32;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        let secs = modified.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                        if secs < cutoff {
                            let _ = std::fs::remove_file(entry.path());
                            purged += 1;
                        }
                    }
                }
            }
        }
        if purged > 0 {
            println!("  Purged {} stale image(s) from cache", purged);
        }
    }

    async fn fetch_and_cache_image(&self, url: &str) -> String {
        if !self.cache_images { return url.to_string(); }
        if let Some(cached) = self.get_image(url) { return cached; }
        let path = Self::image_cache_path(url);
        if let Ok(resp) = reqwest::get(url).await {
            if let Ok(bytes) = resp.bytes().await {
                let _ = std::fs::write(&path, &bytes);
                return format!("/img_cache/{}_{}", url.len(),
                    url.rsplit('/').next().unwrap_or("img")
                        .chars().filter(|c| c.is_alphanumeric() || *c == '.').collect::<String>());
            }
        }
        url.to_string()
    }
}

use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use tower_http::cors::{Any, CorsLayer};

static INDEX_HTML: &str  = include_str!("../index.html");
static MOBILE_HTML: &str = include_str!("../mobile.html");

// ── Config ────────────────────────────────────────────────────────────────────

fn debug_enabled() -> bool {
    env::var("DEBUG").map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes")).unwrap_or(false)
}

macro_rules! debug {
    ($($arg:tt)*) => {
        if debug_enabled() { println!($($arg)*); }
    }
}

fn binary_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn load_config() {
    // Look for config.toml next to the binary, falling back to current directory
    let binary_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    let config_path = binary_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("config.toml"));

    if !config_path.exists() { return; }
    let contents = std::fs::read_to_string(&config_path).unwrap_or_default();
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() { continue; }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim().trim_matches('"');
            if env::var(key).is_err() {
                unsafe { env::set_var(key, val); }
            }
        }
    }
}

fn steam_key()     -> String { env::var("STEAM_API_KEY").unwrap_or_default() }
fn rawg_key()      -> String { env::var("RAWG_API_KEY").unwrap_or_default() }
fn anthropic_key() -> String { env::var("ANTHROPIC_API_KEY").unwrap_or_default() }
fn steam_id()      -> String { env::var("STEAM_ID").unwrap_or_default() }
fn cache_ttl()     -> u64 {
    env::var("CACHE_TTL_DAYS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(7) * 86400
}
fn summary_cache_ttl() -> u64 { 86400 * 30 }
fn library_cache_ttl() -> u64 {
    env::var("LIBRARY_CACHE_TTL_HOURS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(24) * 3600
}
fn hltb_url() -> String { env::var("HLTB_API_URL").unwrap_or_default() }

fn cache_images() -> bool {
    env::var("CACHE_IMAGES")
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false)
}
fn port() -> u16 {
    env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000)
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)] struct GamesQuery   { steamid: String }
#[derive(Deserialize)] struct RawgQuery    { name: String, appid: Option<u64>, slug_override: Option<String>, steam_override: Option<bool> }
#[derive(Deserialize)] struct ReviewsQuery { appid: String }
#[derive(Deserialize)] struct SummaryQuery { appid: String, name: String }
#[derive(Deserialize)] struct PlaytimeQuery { appid: String, name: String }
#[derive(Deserialize)] struct SnoozeQuery    { appid: String, name: Option<String>, days: Option<i64> }
#[derive(Deserialize)] struct SearchQuery        { q: String }
#[derive(Deserialize)] struct ClearCacheQuery    { #[serde(rename = "type")] cache_type: String, name: Option<String> }
#[derive(Deserialize)] struct TrailerOverrideQuery { appid: String, youtube_id: String }
#[derive(Serialize)]   struct ErrorResponse { error: String }

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn index()  -> Html<&'static str> { Html(INDEX_HTML) }
async fn manifest() -> impl IntoResponse {
    let body = serde_json::json!({
        "name": "What Should I Play?",
        "short_name": "Backlog",
        "description": "Discover unplayed games in your Steam library",
        "start_url": "/",
        "display": "standalone",
        "background_color": "#0a0a0c",
        "theme_color": "#0a0a0c",
        "icons": [
            {
                "src": "https://cdn.akamai.steamstatic.com/valvesoftware/images/apps/steam/steam_logo.png",
                "sizes": "192x192",
                "type": "image/png"
            }
        ]
    });
    ([(header::CONTENT_TYPE, "application/manifest+json")], body.to_string())
}

async fn cache_stats(State(cache): State<Cache>) -> Json<serde_json::Value> {
    Json(json!({
        "rawg":     cache.table_count("rawg_cache"),
        "reviews":  cache.table_count("reviews_cache"),
        "library":  cache.table_count("library_cache"),
        "summary":  cache.table_count("summary_cache"),
        "slug_overrides":    cache.table_count("slug_overrides"),
        "trailer_overrides": cache.table_count("trailer_overrides"),
    }))
}

async fn cache_clear(State(cache): State<Cache>, axum::extract::Path(table): axum::extract::Path<String>) -> Json<serde_json::Value> {
    let allowed = ["rawg_cache", "reviews_cache", "library_cache", "summary_cache"];
    if !allowed.contains(&table.as_str()) {
        return Json(json!({ "error": "Cannot clear that table" }));
    }
    let count = cache.clear_table(&table);
    Json(json!({ "cleared": count, "table": table }))
}
async fn mobile() -> Html<&'static str> { Html(MOBILE_HTML) }

async fn get_games(State(cache): State<Cache>, Query(params): Query<GamesQuery>) -> Response {
    let key = steam_key();
    if key.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: "STEAM_API_KEY not configured".into() })).into_response();
    }
    if params.steamid.is_empty() || !params.steamid.chars().all(|c| c.is_ascii_digit()) {
        return (StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "Invalid Steam ID".into() })).into_response();
    }
    if let Some(cached) = cache.get_library(&params.steamid) {
        return Json(cached).into_response();
    }
    let mut q = HashMap::new();
    q.insert("key",                       key.as_str());
    q.insert("steamid",                   &params.steamid);
    q.insert("include_appinfo",           "1");
    q.insert("include_played_free_games", "1");
    q.insert("format",                    "json");

    let client = reqwest::Client::new();
    match client.get("https://api.steampowered.com/IPlayerService/GetOwnedGames/v0001/")
        .query(&q).send().await
    {
        Ok(r) => {
            let status = r.status();
            match r.json::<Value>().await {
                Ok(body) => {
                    if status.is_success() { cache.set_library(&params.steamid, &body); }
                    (StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK), Json(body)).into_response()
                }
                Err(_) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: "Bad Steam response".into() })).into_response(),
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: format!("Steam request failed: {}", e) })).into_response(),
    }
}

async fn fetch_steam_appdetails(client: &reqwest::Client, appid: u64) -> Value {
    if appid == 0 { return Value::Null; }
    let url = format!("https://store.steampowered.com/api/appdetails?appids={}&cc=us&l=en", appid);
    match client.get(&url).send().await {
        Ok(r) => {
            let data: Value = r.json().await.unwrap_or(Value::Null);
            let key = appid.to_string();
            if data[&key]["success"].as_bool().unwrap_or(false) {
                data[&key]["data"].clone()
            } else {
                Value::Null
            }
        }
        Err(_) => Value::Null,
    }
}

fn steam_to_rawg_format(steam: &Value, appid: u64) -> Value {
    let genres: Vec<Value> = steam["genres"].as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|g| json!({ "name": g["description"] }))
        .collect();

    let screenshots: Vec<Value> = steam["screenshots"].as_array()
        .unwrap_or(&vec![])
        .iter()
        .take(8)
        .map(|s| json!({ "image": s["path_full"] }))
        .collect();

    let metacritic = steam["metacritic"]["score"].as_u64();
    let bg = steam["background"].as_str()
        .or_else(|| steam["header_image"].as_str())
        .unwrap_or("");

    json!({
        "name":             steam["name"],
        "description":      steam["detailed_description"],
        "metacritic":       metacritic,
        "released":         steam["release_date"]["date"],
        "playtime":         Value::Null,
        "genres":           genres,
        "tags":             [],
        "background_image": bg,
        "screenshots":      screenshots,
        "trailer":          Value::Null,
        "trailer_preview":  Value::Null,
        "rawg_url":         format!("https://store.steampowered.com/app/{}", appid),
        "website":          steam["website"],
        "ratings_count":    Value::Null,
        "rating":           Value::Null,
    })
}

fn inject_yt_trailer(cache: &Cache, response: &Value, appid: Option<u64>) -> Value {
    if let Some(appid) = appid {
        if response["trailer"].is_null() {
            if let Some(yt_id) = cache.get_trailer_override(&appid.to_string()) {
                let mut r = response.clone();
                r["trailer"] = json!(format!("https://www.youtube.com/embed/{}", yt_id));
                r["trailer_type"] = json!("youtube");
                return r;
            }
        }
    }
    response.clone()
}

async fn get_rawg(State(cache): State<Cache>, Query(params): Query<RawgQuery>) -> Response {
    let key = rawg_key();

    println!("RAWG request for: {}", params.name);

    // Check cache first (unless steam_override requested)
    if !params.steam_override.unwrap_or(false) {
        if let Some(cached) = cache.get_rawg(&params.name) {
            debug!("RAWG cache hit for: {}", params.name);
            let cached = inject_yt_trailer(&cache, &cached, params.appid);
            return Json(cached).into_response();
        }
        debug!("RAWG cache miss for: {}", params.name);
    }

    // If steam_override requested, wipe cache and slug override first, then bypass RAWG
    if params.steam_override.unwrap_or(false) {
        if let Ok(conn) = cache.conn.lock() {
            let _ = conn.execute("DELETE FROM rawg_cache WHERE game_name = ?1", params![&params.name]);
            let _ = conn.execute("DELETE FROM slug_overrides WHERE game_name = ?1", params![&params.name]);
        }
        if let Some(appid) = params.appid {
            let client = reqwest::Client::builder().user_agent("SteamBacklogBrowser/1.0").build().unwrap_or_default();
            let steam = fetch_steam_appdetails(&client, appid).await;
            if !steam.is_null() {
                let response = build_response_steam_primary_skip_rawg(&client, &cache, &steam, appid, params.appid).await;
                cache.set_slug_override(&params.name, &format!("steam:{}", appid));
                cache.set_rawg(&params.name, &response);
                return Json(response).into_response();
            }
        }
    }

    let client = reqwest::Client::builder().user_agent("SteamBacklogBrowser/1.0").build().unwrap_or_default();

    // Check permanent slug override
    let override_slug = cache.get_slug_override(&params.name);
    let effective_override = params.slug_override.as_deref()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or(override_slug);

    if let Some(ref slug) = effective_override {
        if !slug.is_empty() {
            // Handle steam: override
            if slug.starts_with("steam:") {
                let appid: u64 = slug.trim_start_matches("steam:").parse().unwrap_or(0);
                if appid > 0 {
                    let steam = fetch_steam_appdetails(&client, appid).await;
                    if !steam.is_null() {
                        let response = build_response_steam_primary(&client, &cache, &steam, appid, &key, &params.name, params.appid).await;
                        cache.set_rawg(&params.name, &response);
                        return Json(response).into_response();
                    }
                }
            } else {
                // RAWG slug override — fetch trailer/metacritic from RAWG, everything else from Steam
                if let Some(appid) = params.appid {
                    let steam = fetch_steam_appdetails(&client, appid).await;
                    if !steam.is_null() {
                        let response = build_response_steam_primary(&client, &cache, &steam, appid, &key, &params.name, params.appid).await;
                        if params.slug_override.as_deref().filter(|s| !s.is_empty()).is_some() {
                            cache.set_slug_override(&params.name, slug);
                        }
                        cache.set_rawg(&params.name, &response);
                        return Json(response).into_response();
                    }
                }
            }
        }
    }

    // Primary: Steam appdetails
    let appid = match params.appid {
        Some(id) => id,
        None => return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "appid required".into() })).into_response(),
    };

    let steam = fetch_steam_appdetails(&client, appid).await;
    if steam.is_null() {
        return (StatusCode::NOT_FOUND, Json(ErrorResponse { error: "Game not found on Steam".into() })).into_response();
    }

    let response = build_response_steam_primary(&client, &cache, &steam, appid, &key, &params.name, params.appid).await;
    cache.set_rawg(&params.name, &response);
    Json(response).into_response()
}

// Steam-only response with no RAWG lookup at all (used for Wrong game? override)
async fn build_response_steam_primary_skip_rawg(
    client: &reqwest::Client,
    cache: &Cache,
    steam: &Value,
    appid: u64,
    appid_opt: Option<u64>,
) -> Value {
    let screenshots: Vec<Value> = steam["screenshots"].as_array()
        .unwrap_or(&vec![])
        .iter().take(8)
        .map(|s| json!({ "image": s["path_full"] }))
        .collect();
    let genres: Vec<Value> = steam["genres"].as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|g| json!({ "name": g["description"] }))
        .collect();
    let bg = format!("https://cdn.akamai.steamstatic.com/steam/apps/{}/library_hero.jpg", appid);
    let response = json!({
        "name":             steam["name"],
        "description":      steam["detailed_description"],
        "metacritic":       steam["metacritic"]["score"],
        "released":         steam["release_date"]["date"],
        "playtime":         Value::Null,
        "genres":           genres,
        "tags":             [],
        "background_image": bg,
        "screenshots":      screenshots,
        "trailer":          Value::Null,
        "trailer_preview":  Value::Null,
        "rawg_url":         format!("https://store.steampowered.com/app/{}", appid),
        "website":          steam["website"],
        "ratings_count":    Value::Null,
        "rating":           Value::Null,
    });
    inject_yt_trailer(cache, &response, appid_opt)
}

// Build final response using Steam as primary, RAWG only for metacritic/playtime/trailer
async fn build_response_steam_primary(
    client: &reqwest::Client,
    cache: &Cache,
    steam: &Value,
    appid: u64,
    rawg_key: &str,
    name: &str,
    appid_opt: Option<u64>,
) -> Value {
    // Search RAWG for metacritic/playtime/trailer only
    // Check if this is a steam-only override — if so skip RAWG entirely
    let is_steam_override = cache.get_slug_override(name)
        .map(|s| s.starts_with("steam:"))
        .unwrap_or(false);

    let (metacritic, playtime, trailer, trailer_preview, rawg_bg, rawg_screenshots, rawg_slug, metacritic_url) = if !rawg_key.is_empty() && !is_steam_override {
        let slug = name.to_lowercase()
            .chars().map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-').filter(|s| !s.is_empty())
            .collect::<Vec<_>>().join("-");

        // Try direct slug first
        let direct_url = format!("https://api.rawg.io/api/games/{}?key={}", slug, rawg_key);
        let direct: Value = match client.get(&direct_url).send().await {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
            _ => Value::Null,
        };

        let rawg_detail = if direct["slug"].is_string() {
            direct
        } else {
            // Search fallback
            let search_url = format!(
                "https://api.rawg.io/api/games?key={}&search={}&page_size=5&stores=1&platforms=4",
                rawg_key, urlencoding::encode(name)
            );
            let search_data: Value = client.get(&search_url).send().await
                .ok().and_then(|r| futures_executor_block_on(r.json()).ok())
                .unwrap_or(Value::Null);
            let results = search_data["results"].as_array().cloned().unwrap_or_default();
            if results.is_empty() {
                Value::Null
            } else {
                let rawg_name = results[0]["name"].as_str().unwrap_or("").to_lowercase();
                let clean = |s: &str| -> String {
                    s.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace())
                     .collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
                };
                let is_match = {
                    let rc = clean(&rawg_name);
                    let sc = clean(&name.to_lowercase());
                    let fw = sc.split_whitespace().next().unwrap_or("").to_string();
                    rc == sc || rc.contains(&sc) || sc.contains(&rc)
                        || (!fw.is_empty() && fw.len() > 3 && rc.starts_with(&fw))
                };
                if is_match {
                    let detail_url = format!("https://api.rawg.io/api/games/{}?key={}", results[0]["slug"].as_str().unwrap_or(""), rawg_key);
                    client.get(&detail_url).send().await
                        .ok().and_then(|r| futures_executor_block_on(r.json()).ok())
                        .unwrap_or(Value::Null)
                } else { Value::Null }
            }
        };

        // Get trailer if we have a valid rawg detail
        let (tr, tr_prev) = if rawg_detail["slug"].is_string() {
            let game_id = rawg_detail["id"].as_u64().unwrap_or(0);
            let trailer_url = format!("https://api.rawg.io/api/games/{}/movies?key={}", game_id, rawg_key);
            let trailers: Value = client.get(&trailer_url).send().await
                .ok().and_then(|r| futures_executor_block_on(r.json()).ok())
                .unwrap_or(json!({"results":[]}));
            (
                trailers["results"][0]["data"]["max"].as_str().map(|s| json!(s)).unwrap_or(Value::Null),
                trailers["results"][0]["preview"].as_str().map(|s| json!(s)).unwrap_or(Value::Null),
            )
        } else { (Value::Null, Value::Null) };

        let rawg_name_found = rawg_detail["name"].as_str().unwrap_or("").to_lowercase();
        let clean = |s: &str| -> String {
            s.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace())
             .collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let name_matches = {
            let rc = clean(&rawg_name_found);
            let sc = clean(&name.to_lowercase());
            let fw = sc.split_whitespace().next().unwrap_or("").to_string();
            rc == sc || rc.contains(&sc) || sc.contains(&rc)
                || (!fw.is_empty() && fw.len() > 3 && rc.starts_with(&fw))
        };
        // Only use RAWG background/screenshots if name matches
        let rawg_bg = if name_matches { rawg_detail["background_image"].clone() } else { Value::Null };
        let rawg_shots = if name_matches { rawg_detail["screenshots"].clone() } else { Value::Null };
        let rawg_slug = rawg_detail["slug"].as_str().unwrap_or("").to_string();
        (
            rawg_detail["metacritic"].clone(),
            rawg_detail["playtime"].clone(),
            tr,
            tr_prev,
            rawg_bg,
            rawg_shots,
            rawg_slug,
            rawg_detail["metacritic_url"].clone(),
        )
    } else {
        (Value::Null, Value::Null, Value::Null, Value::Null, Value::Null, Value::Null, String::new(), Value::Null)
    };

    // Metacritic from Steam if RAWG didn't have it
    let metacritic = if metacritic.is_null() {
        steam["metacritic"]["score"].clone()
    } else { metacritic };

    // Build screenshots from Steam
    let screenshots: Vec<Value> = steam["screenshots"].as_array()
        .unwrap_or(&vec![])
        .iter().take(8)
        .map(|s| json!({ "image": s["path_full"] }))
        .collect();

    let genres: Vec<Value> = steam["genres"].as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|g| json!({ "name": g["description"] }))
        .collect();

    // Priority: Steam library_hero (always correct) → RAWG bg → Steam background → header
    let library_hero = format!("https://cdn.akamai.steamstatic.com/steam/apps/{}/library_hero.jpg", appid);
    let bg = if !library_hero.is_empty() { library_hero }
    else if let Some(rawg) = rawg_bg.as_str().filter(|s| !s.is_empty()) { rawg.to_string() }
    else {
        steam["background"].as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| steam["header_image"].as_str())
            .unwrap_or("").to_string()
    };

    // Use RAWG screenshots if available (better quality/selection), otherwise Steam
    let final_screenshots = if rawg_screenshots.is_array() && !rawg_screenshots.as_array().unwrap().is_empty() {
        rawg_screenshots
    } else {
        json!(screenshots)
    };

    let response = json!({
        "name":             steam["name"],
        "description":      steam["detailed_description"],
        "metacritic":       metacritic,
        "released":         steam["release_date"]["date"],
        "playtime":         playtime,
        "genres":           genres,
        "tags":             [],
        "background_image": bg,
        "screenshots":      final_screenshots,
        "trailer":          trailer,
        "trailer_preview":  trailer_preview,
        "metacritic_url":   metacritic_url,
        "rawg_url":         if rawg_slug.is_empty() {
                                json!(format!("https://store.steampowered.com/app/{}", appid))
                            } else {
                                json!(format!("https://rawg.io/games/{}", rawg_slug))
                            },
        "website":          steam["website"],
        "ratings_count":    Value::Null,
        "rating":           Value::Null,
    });

    inject_yt_trailer(cache, &response, appid_opt)
}

// Blocking json helper for use inside async context indirectly
fn futures_executor_block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

async fn get_reviews(State(cache): State<Cache>, Query(params): Query<ReviewsQuery>) -> Response {
    if params.appid.is_empty() || !params.appid.chars().all(|c| c.is_ascii_digit()) {
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse { error: "Invalid appid".into() })).into_response();
    }
    if let Some(cached) = cache.get_reviews(&params.appid) {
        return Json(cached).into_response();
    }
    let url = format!(
        "https://store.steampowered.com/appreviews/{}?json=1&language=all&purchase_type=all&num_per_page=0",
        params.appid
    );
    let client = reqwest::Client::builder().user_agent("SteamBacklogBrowser/1.0").build().unwrap_or_default();
    match client.get(&url).send().await {
        Ok(r) => match r.json::<Value>().await {
            Ok(body) => {
                let summary = &body["query_summary"];
                let response = json!({
                    "total_positive":    summary["total_positive"],
                    "total_negative":    summary["total_negative"],
                    "total_reviews":     summary["total_reviews"],
                    "review_score_desc": summary["review_score_desc"],
                    "reviews_url":       format!("https://store.steampowered.com/app/{}/#app_reviews_hash", params.appid),
                });
                cache.set_reviews(&params.appid, &response);
                Json(response).into_response()
            }
            Err(_) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: "Bad Steam review response".into() })).into_response(),
        },
        Err(e) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: format!("Steam reviews failed: {}", e) })).into_response(),
    }
}

/// POST /api/trailer_override  — save a YouTube video ID for a game
async fn set_trailer_override(State(cache): State<Cache>, Query(params): Query<TrailerOverrideQuery>) -> Response {
    if params.appid.is_empty() || params.youtube_id.is_empty() {
        return (StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "Missing appid or youtube_id".into() })).into_response();
    }
    cache.set_trailer_override(&params.appid, &params.youtube_id);
    Json(json!({ "ok": true })).into_response()
}

/// GET /api/trailer_override?appid=XXXXX — get saved YouTube video ID
async fn get_trailer_override_handler(State(cache): State<Cache>, Query(params): Query<GamesQuery>) -> Response {
    match cache.get_trailer_override(&params.steamid) {
        Some(id) => Json(json!({ "youtube_id": id })).into_response(),
        None => Json(json!({ "youtube_id": null })).into_response(),
    }
}

async fn clear_cache(State(cache): State<Cache>, Query(params): Query<ClearCacheQuery>) -> Response {
    if let Ok(conn) = cache.conn.lock() {
        match params.cache_type.as_str() {
            "rawg"     => { let _ = conn.execute("DELETE FROM rawg_cache", []); }
            "rawg_single" => {
                if let Some(ref name) = params.name {
                    let _ = conn.execute("DELETE FROM rawg_cache WHERE game_name = ?1", params![name]);
                    let _ = conn.execute("DELETE FROM slug_overrides WHERE game_name = ?1", params![name]);
                }
            }
            "reviews"  => { let _ = conn.execute("DELETE FROM reviews_cache", []); }
            "library"  => { let _ = conn.execute("DELETE FROM library_cache", []); }
            "summary"  => { let _ = conn.execute("DELETE FROM summary_cache", []); }
            "playtime" => { let _ = conn.execute("DELETE FROM playtime_cache", []); }
            "hltb"     => { let _ = conn.execute("DELETE FROM hltb_cache", []); }
            "all"      => {
                let _ = conn.execute("DELETE FROM rawg_cache", []);
                let _ = conn.execute("DELETE FROM reviews_cache", []);
                let _ = conn.execute("DELETE FROM library_cache", []);
                // Intentionally keep: summary_cache, slug_overrides, trailer_overrides, playtime_cache
            }
            _ => return (StatusCode::BAD_REQUEST, Json(json!({"error": "Unknown cache type"}))).into_response(),
        }
    }
    Json(json!({"ok": true})).into_response()
}

async fn search_games(Query(params): Query<SearchQuery>) -> Response {
    if params.q.trim().is_empty() {
        return (StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: "Empty query".into() })).into_response();
    }

    let client = reqwest::Client::builder()
        .user_agent("SteamBacklogBrowser/1.0")
        .build()
        .unwrap_or_default();

    // Search Steam store directly — returns appids natively
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&cc=us&l=en",
        urlencoding::encode(params.q.trim())
    );

    match client.get(&url).send().await {
        Ok(r) => match r.json::<Value>().await {
            Ok(data) => {
                let empty = vec![];
                let items = data["items"].as_array().unwrap_or(&empty);
                let dlc_keywords = ["deluxe edition", "upgrade", "bundle", "soundtrack", "dlc", 
                    "season pass", "pack", "outfit", "skin", "cosmetic", "artbook",
                    "supporter", "donation", "charity", "demo", "beta"];
                let results: Vec<Value> = items.iter()
                    .filter_map(|g| {
                        let appid = g["id"].as_u64()?;
                        let name = g["name"].as_str().unwrap_or("").to_lowercase();
                        // Filter out DLC and non-game items
                        if dlc_keywords.iter().any(|kw| name.contains(kw)) {
                            return None;
                        }
                        Some(json!({
                            "name":      g["name"],
                            "appid":     appid,
                            "tiny_image": g["tiny_image"],
                            "metascore": g["metascore"],
                        }))
                    })
                    .take(8)
                    .collect();
                Json(json!(results)).into_response()
            }
            Err(_) => (StatusCode::BAD_GATEWAY,
                Json(ErrorResponse { error: "Bad Steam search response".into() })).into_response(),
        },
        Err(e) => (StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: format!("Steam search failed: {}", e) })).into_response(),
    }
}

async fn get_game_image_handler(State(cache): State<Cache>, Query(params): Query<ReviewsQuery>) -> Response {
    // Check cache first
    if let Some(url) = cache.get_game_image(&params.appid) {
        return Json(json!({ "url": url })).into_response();
    }
    // Fetch from Steam appdetails
    let client = reqwest::Client::builder()
        .user_agent("SteamBacklogBrowser/1.0")
        .build()
        .unwrap_or_default();
    let appid: u64 = params.appid.parse().unwrap_or(0);
    let steam = fetch_steam_appdetails(&client, appid).await;
    if !steam.is_null() {
        let url = steam["header_image"].as_str().unwrap_or("");
        if !url.is_empty() {
            cache.set_game_image(&params.appid, url);
            return Json(json!({ "url": url })).into_response();
        }
    }
    Json(json!({ "url": null })).into_response()
}

async fn get_hltb_data(State(cache): State<Cache>, Query(params): Query<ReviewsQuery>) -> Response {
    let base = hltb_url();
    if base.is_empty() {
        return Json(json!({ "available": false })).into_response();
    }
    if let Some(cached) = cache.get_hltb(&params.appid) {
        return Json(cached).into_response();
    }
    let client = reqwest::Client::builder()
        .user_agent("SteamBacklogBrowser/1.0")
        .build()
        .unwrap_or_default();
    let url = format!("{}/game?steam_id={}", base.trim_end_matches('/'), params.appid);
    match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            match r.json::<Value>().await {
                Ok(data) => {
                    // Map to our format
                    let response = json!({
                        "available":      true,
                        "main":           data["main_story"],
                        "extras":         data["main_extra"],
                        "completionist":  data["completionist"],
                        "source":         data["source"],
                        "title":          data["title"],
                    });
                    // Only cache if we have real data
                    if data["main_story"].is_number() || data["main_extra"].is_number() || data["completionist"].is_number() {
                        cache.set_hltb(&params.appid, &response);
                    }
                    Json(response).into_response()
                }
                Err(_) => Json(json!({ "available": false })).into_response(),
            }
        }
        _ => Json(json!({ "available": false })).into_response(),
    }
}

async fn get_snoozed(State(cache): State<Cache>) -> Response {
    let now = Cache::now();
    if let Ok(conn) = cache.conn.lock() {
        let mut stmt = match conn.prepare(
            "SELECT appid, name, wake_date FROM snoozed ORDER BY wake_date ASC"
        ) {
            Ok(s) => s,
            Err(_) => return Json(json!({ "snoozed": [], "due": [] })).into_response(),
        };
        let rows: Vec<Value> = {
            let mapped = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            });
            match mapped {
                Ok(m) => m.filter_map(|r| r.ok()).map(|(appid, name, wake_date)| json!({
                    "appid": appid,
                    "name": name,
                    "wake_date": wake_date,
                    "due": (wake_date as u64) <= now,
                    "days_left": if (wake_date as u64) > now { ((wake_date as u64 - now) / 86400) as i64 } else { 0 },
                })).collect(),
                Err(_) => vec![],
            }
        };

        let due: Vec<&Value> = rows.iter().filter(|r| r["due"].as_bool().unwrap_or(false)).collect();
        let snoozed: Vec<&Value> = rows.iter().filter(|r| !r["due"].as_bool().unwrap_or(false)).collect();
        return Json(json!({ "snoozed": snoozed, "due": due })).into_response();
    }
    Json(json!({ "snoozed": [], "due": [] })).into_response()
}

async fn set_snooze(State(cache): State<Cache>, Query(params): Query<SnoozeQuery>) -> Response {
    let days = params.days.unwrap_or(30);
    let wake = Cache::now() + (days as u64 * 86400);
    if let Ok(conn) = cache.conn.lock() {
        let _ = conn.execute(
            "INSERT OR REPLACE INTO snoozed (appid, name, wake_date) VALUES (?1, ?2, ?3)",
            params![params.appid, params.name.unwrap_or_default(), wake as i64],
        );
    }
    Json(json!({ "ok": true })).into_response()
}

async fn remove_snooze(State(cache): State<Cache>, Query(params): Query<SnoozeQuery>) -> Response {
    if let Ok(conn) = cache.conn.lock() {
        let _ = conn.execute("DELETE FROM snoozed WHERE appid = ?1", params![params.appid]);
    }
    Json(json!({ "ok": true })).into_response()
}

async fn get_playtime(State(cache): State<Cache>, Query(params): Query<PlaytimeQuery>) -> Response {
    let anthropic = anthropic_key();
    if anthropic.is_empty() {
        return (StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse { error: "ANTHROPIC_API_KEY not configured".into() })).into_response();
    }
    if let Some(cached) = cache.get_playtime(&params.appid) {
        return Json(cached).into_response();
    }
    let client = reqwest::Client::builder()
        .user_agent("SteamBacklogBrowser/1.0")
        .build()
        .unwrap_or_default();

    let mut prompt = String::from("How long does it take to beat ");
    prompt.push_str(&params.name);
    prompt.push_str("? Search HowLongToBeat or similar sites. Return ONLY a raw JSON object with keys: main, extras, completionist, source. Values are strings like 15 hours or null if unknown. No markdown, no explanation, just the JSON object.");

    let result = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &anthropic)
        .header("anthropic-version", "2023-06-01")
        .header(header::CONTENT_TYPE, "application/json")
        .json(&json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 300,
            "tools": [{ "type": "web_search_20250305", "name": "web_search" }],
            "messages": [{ "role": "user", "content": prompt }]
        }))
        .send()
        .await;

    match result {
        Ok(r) if r.status().is_success() => {
            match r.json::<Value>().await {
                Ok(data) => {
                    let text = data["content"]
                        .as_array()
                        .map(|arr| arr.iter()
                            .filter_map(|b| b["text"].as_str())
                            .collect::<Vec<_>>()
                            .join(""))
                        .unwrap_or_default();
                    // Try to extract JSON object from anywhere in the response
                    let parsed = text.find('{').and_then(|start| {
                        text.rfind('}').map(|end| &text[start..=end])
                    }).and_then(|json_str| serde_json::from_str::<Value>(json_str).ok());

                    match parsed {
                        Some(p) => {
                            let main_ok = p["main"].as_str().map(|s| !s.eq_ignore_ascii_case("null") && !s.is_empty()).unwrap_or(false);
                            let extras_ok = p["extras"].as_str().map(|s| !s.eq_ignore_ascii_case("null") && !s.is_empty()).unwrap_or(false);
                            let comp_ok = p["completionist"].as_str().map(|s| !s.eq_ignore_ascii_case("null") && !s.is_empty()).unwrap_or(false);
                            let has_data = main_ok || extras_ok || comp_ok;
                            if has_data {
                                cache.set_playtime(&params.appid, &p);
                                Json(p).into_response()
                            } else {
                                (StatusCode::NOT_FOUND,
                                    Json(ErrorResponse { error: "No playtime data found for this game".into() })).into_response()
                            }
                        }
                        None => (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse { error: "No playtime data found".into() })).into_response(),
                    }
                }
                Err(_) => (StatusCode::BAD_GATEWAY,
                    Json(ErrorResponse { error: "Bad Anthropic response".into() })).into_response(),
            }
        }
        Ok(r) => (StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: format!("Anthropic error: {}", r.status()) })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: format!("Request failed: {}", e) })).into_response(),
    }
}

async fn get_summary(State(cache): State<Cache>, Query(params): Query<SummaryQuery>) -> Response {
    let anthropic = anthropic_key();
    if anthropic.is_empty() {
        return (StatusCode::NOT_IMPLEMENTED,
            Json(ErrorResponse { error: "ANTHROPIC_API_KEY not configured".into() })).into_response();
    }
    if let Some(cached) = cache.get_summary(&params.appid) {
        return Json(json!({ "summary": cached })).into_response();
    }

    let reviews_url = format!(
        "https://store.steampowered.com/appreviews/{}?json=1&language=english&purchase_type=all&num_per_page=25&filter=recent",
        params.appid
    );
    let client = reqwest::Client::builder().user_agent("SteamBacklogBrowser/1.0").build().unwrap_or_default();
    let reviews_data: Value = match client.get(&reviews_url).send().await {
        Ok(r) => r.json().await.unwrap_or(Value::Null),
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: format!("Steam request failed: {}", e) })).into_response(),
    };

    let reviews = match reviews_data["reviews"].as_array() {
        Some(r) if !r.is_empty() => r.clone(),
        _ => return (StatusCode::NOT_FOUND, Json(ErrorResponse { error: "No reviews found".into() })).into_response(),
    };

    let review_texts: Vec<String> = reviews.iter().take(25).map(|r| {
        let voted_up = r["voted_up"].as_bool().unwrap_or(false);
        let text = r["review"].as_str().unwrap_or("").chars().take(400).collect::<String>();
        format!("{} {}", if voted_up { "👍" } else { "👎" }, text)
    }).collect();

    let review_block = review_texts.join("\n\n");
    let prompt = format!(
        "You are summarizing Steam user reviews for the game {}. Here are {} recent reviews:\n\n{}\n\nWrite a concise 3-4 sentence summary of what players think. Cover what they enjoy most, any common criticisms, and who the game is best suited for. Be specific and direct. Do not start with Players or use generic phrases. Do not include any headers or titles.",
        params.name, review_texts.len(), review_block
    );

    let result = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &anthropic)
        .header("anthropic-version", "2023-06-01")
        .header(header::CONTENT_TYPE, "application/json")
        .json(&json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 300,
            "messages": [{ "role": "user", "content": prompt }]
        }))
        .send()
        .await;

    match result {
        Ok(r) if r.status().is_success() => {
            match r.json::<Value>().await {
                Ok(data) => {
                    let raw = data["content"][0]["text"].as_str().unwrap_or("").to_string();
                    // Strip any markdown headers Claude adds
                    let summary = raw.lines()
                        .filter(|l| !l.trim_start().starts_with('#'))
                        .collect::<Vec<_>>()
                        .join("
")
                        .trim()
                        .to_string();
                    if !summary.is_empty() {
                        cache.set_summary(&params.appid, &summary);
                        Json(json!({ "summary": summary })).into_response()
                    } else {
                        (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "Empty summary returned".into() })).into_response()
                    }
                }
                Err(_) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: "Bad Anthropic response".into() })).into_response(),
            }
        }
        Ok(r) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: format!("Anthropic error: {}", r.status()) })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(ErrorResponse { error: format!("Anthropic request failed: {}", e) })).into_response(),
    }
}

async fn serve_image(axum::extract::Path(filename): axum::extract::Path<String>) -> Response {
    let path = format!("image_cache/{}", filename);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mime = if filename.ends_with(".jpg") || filename.ends_with(".jpeg") { "image/jpeg" }
                       else if filename.ends_with(".png") { "image/png" }
                       else if filename.ends_with(".webp") { "image/webp" }
                       else { "application/octet-stream" };
            (StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn get_config() -> Json<serde_json::Value> {
    Json(json!({ "steam_id": steam_id() }))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    load_config();

    let cors  = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);
    let cache = Cache::new(cache_ttl(), library_cache_ttl(), cache_images());
    cache.purge_stale_images();

    let app = Router::new()
        .route("/",                        get(index))
        .route("/mobile",                  get(mobile))
        .route("/manifest.json",           get(manifest))
        .route("/api/cache/stats",         get(cache_stats))
        .route("/api/cache/clear/:table",  get(cache_clear))
        .route("/api/games",               get(get_games))
        .route("/api/rawg",                get(get_rawg))
        .route("/api/reviews",             get(get_reviews))
        .route("/api/search",              get(search_games))
        .route("/api/cache/clear",         post(clear_cache))
        .route("/api/trailer_override",    get(get_trailer_override_handler).post(set_trailer_override))
        .route("/api/summary",             get(get_summary))
        .route("/api/playtime",            get(get_playtime))
        .route("/api/snooze",              get(get_snoozed).post(set_snooze).delete(remove_snooze))
        .route("/api/hltb",                get(get_hltb_data))
        .route("/api/game_image",          get(get_game_image_handler))
        .route("/api/config",              get(get_config))
        .route("/img_cache/:filename",     get(serve_image))
        .with_state(cache)
        .layer(cors);

    let addr = format!("0.0.0.0:{}", port());
    println!("steam-backlog listening on http://localhost:{}", port());
    println!("  STEAM_API_KEY: {}", if steam_key().is_empty()     { "NOT SET" } else { "set" });
    println!("  RAWG_API_KEY:  {}", if rawg_key().is_empty()      { "NOT SET" } else { "set" });
    println!("  ANTHROPIC:     {}", if anthropic_key().is_empty() { "not set (summaries disabled)" } else { "set" });
    let hltb = hltb_url();
    println!("  HLTB_API:      {}", if hltb.is_empty() { "not set (HLTB disabled)".to_string() } else { hltb });
    println!("  CACHE_TTL:     {} days", cache_ttl() / 86400);
    println!("  LIBRARY_CACHE: {} hours", library_cache_ttl() / 3600);
    println!("  CACHE_IMAGES:  {}", cache_images());

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
