//! hltb - HowLongToBeat lookup CLI + optional HTTP server
//!
//! CLI usage:
//!   hltb --steam-id 1145360
//!   hltb --title "Cyberpunk 2077"
//!   hltb --steam-id 1145360 --force
//!
//! Server mode:
//!   hltb --serve
//!   curl http://localhost:9234/game?steam_id=1145360
//!   curl http://localhost:9234/game?title=Hades

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

static VERBOSE: AtomicBool = AtomicBool::new(true);

macro_rules! veprintln {
    ($($arg:tt)*) => {
        if VERBOSE.load(Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
}
use clap::Parser;
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process;
use wreq::Client;
use wreq_util::Emulation;

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "hltb", about = "HowLongToBeat lookup", version = "1.0.0")]
struct Cli {
    #[arg(long, conflicts_with = "title")]
    steam_id: Option<u64>,

    #[arg(long, conflicts_with = "steam_id")]
    title: Option<String>,

    #[arg(long, short)]
    force: bool,

    #[arg(long, short)]
    pretty: bool,

    #[arg(long)]
    serve: bool,

    #[arg(long)]
    config: Option<PathBuf>,
}

// ─── Config ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone)]
struct Config {
    search: SearchConfig,
    #[serde(default)]
    claude: ClaudeConfig,
    #[serde(default)]
    server: ServerConfig,
}

#[derive(Deserialize, Clone)]
struct SearchConfig {
    api_key: String,
}

#[derive(Deserialize, Clone, Default)]
struct ClaudeConfig {
    api_key: Option<String>,
    #[serde(default = "default_true")]
    fallback_enabled: bool,
}

#[derive(Deserialize, Clone)]
struct ServerConfig {
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_true")]
    verbose: bool,
}

fn default_port() -> u16 { 9234 }
fn default_true() -> bool { true }

impl Default for ServerConfig {
    fn default() -> Self { Self { port: default_port(), verbose: true } }
}

fn config_path(cli_override: Option<&PathBuf>) -> PathBuf {
    if let Some(p) = cli_override { return p.clone(); }
    // Look next to the binary first (same dir as executable)
    if let Ok(exe) = std::env::current_exe() {
        let next_to_bin = exe.parent().unwrap_or(std::path::Path::new(".")).join("config.toml");
        if next_to_bin.exists() { return next_to_bin; }
    }
    // Fallback: current working directory
    PathBuf::from("config.toml")
}

fn load_config(path: &PathBuf) -> Result<Config, String> {
    if !path.exists() {
        return Err(format!(
            "config file not found at {}\n\
             \n\
             Create it:\n\
             \n\
             mkdir -p ~/.config/hltb\n\
             cp config.example.toml ~/.config/hltb/config.toml",
            path.display()
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read config: {e}"))?;
    toml::from_str(&text)
        .map_err(|e| format!("failed to parse config: {e}"))
}

// ─── SQLite cache ─────────────────────────────────────────────────────────────

fn db_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        return exe.parent().unwrap_or(std::path::Path::new(".")).join("cache.db");
    }
    PathBuf::from("cache.db")
}

fn open_db() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create cache dir: {e}"))?;
    }
    let conn = Connection::open(&path)
        .map_err(|e| format!("failed to open cache db: {e}"))?;
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS games (
            steam_id    INTEGER PRIMARY KEY,
            hltb_id     INTEGER,
            title       TEXT,
            main_story  REAL,
            main_extra  REAL,
            completionist REAL,
            source      TEXT,
            cached_at   DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_hltb_id ON games(hltb_id);
    ").map_err(|e| format!("failed to init db: {e}"))?;
    Ok(conn)
}

struct CacheRow {
    hltb_id: Option<u64>,
    title: String,
    main_story: Option<f64>,
    main_extra: Option<f64>,
    completionist: Option<f64>,
    source: String,
}

fn cache_get(conn: &Connection, steam_id: u64) -> Option<CacheRow> {
    conn.query_row(
        "SELECT hltb_id, title, main_story, main_extra, completionist, source
         FROM games WHERE steam_id = ?1",
        params![steam_id as i64],
        |row| Ok(CacheRow {
            hltb_id:       row.get::<_, Option<i64>>(0)?.map(|v| v as u64),
            title:         row.get(1)?,
            main_story:    row.get(2)?,
            main_extra:    row.get(3)?,
            completionist: row.get(4)?,
            source:        row.get(5)?,
        }),
    ).ok()
}

fn cache_get_by_hltb_id(conn: &Connection, hltb_id: u64) -> Option<CacheRow> {
    let hltb_id_i = hltb_id as i64;
    conn.query_row(
        "SELECT hltb_id, title, main_story, main_extra, completionist, source
         FROM games WHERE hltb_id = ?1 LIMIT 1",
        params![hltb_id_i],
        |row| Ok(CacheRow {
            hltb_id:       row.get::<_, Option<i64>>(0)?.map(|v| v as u64),
            title:         row.get(1)?,
            main_story:    row.get(2)?,
            main_extra:    row.get(3)?,
            completionist: row.get(4)?,
            source:        row.get(5)?,
        }),
    ).ok()
}

fn cache_set(conn: &Connection, steam_id: u64, result: &GameResult) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO games
         (steam_id, hltb_id, title, main_story, main_extra, completionist, source, cached_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)",
        params![
            steam_id as i64,
            result.hltb_id.map(|v| v as i64),
            result.title,
            result.main_story,
            result.main_extra,
            result.completionist,
            result.source,
        ],
    ).map_err(|e| format!("failed to write cache: {e}"))?;
    Ok(())
}

// ─── Output type ──────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct GameResult {
    steam_id: Option<u64>,
    hltb_id: Option<u64>,
    title: String,
    source: String,       // "hltb" | "claude_fallback"
    cached: bool,
    main_story: Option<f64>,
    main_extra: Option<f64>,
    completionist: Option<f64>,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn sec_to_hours(secs: f64) -> Option<f64> {
    if secs <= 0.0 { None } else { Some((secs / 3600.0 * 10.0).round() / 10.0) }
}

fn extract_times(g: &Value) -> (Option<f64>, Option<f64>, Option<f64>) {
    let f = |k: &str| g.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
    // Game page uses _avg; search API uses base key
    let avg = |base: &str| { let v = f(&format!("{base}_avg")); if v > 0.0 { v } else { f(base) } };
    (
        sec_to_hours(avg("comp_main")),
        sec_to_hours(avg("comp_plus")),
        sec_to_hours(avg("comp_100")),
    )
}

// ─── HTTP client ─────────────────────────────────────────────────────────────

fn make_client() -> Client {
    Client::builder()
        .emulation(Emulation::Chrome130)
        .build()
        .expect("failed to build HTTP client")
}

// ─── Bing search ─────────────────────────────────────────────────────────────

async fn bing_search_hltb_id(client: &Client, api_key: &str, query: &str) -> Result<u64, String> {
    let search_query = format!("site:howlongtobeat.com {query}");
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count=5",
        urlencoding::encode(&search_query)
    );
    veprintln!("[hltb] brave search: {query}");

    let resp = client.get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", api_key)
        .send().await
        .map_err(|e| format!("Bing request failed: {e}"))?;

    let status = resp.status();
    let json: Value = resp.json().await
        .map_err(|e| format!("Bing response parse failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Bing returned HTTP {status}: {json}"));
    }

    let re = Regex::new(r"howlongtobeat\.com/game/(\d+)").unwrap();
    let pages = json.pointer("/web/results")
        .and_then(|v| v.as_array())
        .ok_or("no web results in Brave response")?;

    for page in pages {
        let url_str = page.get("url").and_then(|u| u.as_str()).unwrap_or("");
        if let Some(caps) = re.captures(url_str) {
            let id: u64 = caps[1].parse().unwrap();
            veprintln!("[hltb] found hltb id: {id}");
            return Ok(id);
        }
    }

    Err(format!("no HLTB game page found in Brave results for \"{query}\""))
}

// ─── Bing search for fallback snippets ───────────────────────────────────────

async fn bing_search_snippets(client: &Client, api_key: &str, query: &str) -> Result<String, String> {
    let search_query = format!("how long to beat {query} site:reddit.com OR site:ign.com OR site:metacritic.com OR site:steampowered.com");
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count=5",
        urlencoding::encode(&search_query)
    );
    veprintln!("[hltb] brave fallback search: {query}");

    let resp = client.get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", api_key)
        .send().await
        .map_err(|e| format!("Bing fallback request failed: {e}"))?;

    let json: Value = resp.json().await
        .map_err(|e| format!("Bing fallback parse failed: {e}"))?;

    let pages = json.pointer("/web/results")
        .and_then(|v| v.as_array())
        .ok_or("no fallback results")?;

    // Collect titles + snippets into one block of text for Claude to read
    let text: Vec<String> = pages.iter().map(|p| {
        let title   = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = p.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
        let url     = p.get("url").and_then(|v| v.as_str()).unwrap_or("");
        format!("Source: {url}\n{title}\n{snippet}")
    }).collect();

    Ok(text.join("\n\n---\n\n"))
}

// ─── Claude fallback ──────────────────────────────────────────────────────────

async fn claude_fallback(
    client: &Client,
    api_key: &str,
    game_title: &str,
    snippets: &str,
    steam_id: Option<u64>,
) -> Result<GameResult, String> {
    veprintln!("[hltb] trying claude fallback for: {game_title}");

    let prompt = format!(
        "Based only on the following search result snippets, extract how long it takes to beat \
         the game \"{game_title}\". Return ONLY a JSON object with these exact fields:\n\
         {{\"main_story\": <hours as float or null>, \"main_extra\": <hours as float or null>, \
         \"completionist\": <hours as float or null>}}\n\
         Use null for any category you cannot find reliable data for.\n\
         Do not include any other text, explanation, or markdown.\n\n\
         Snippets:\n{snippets}"
    );

    let body = json!({
        "model": "claude-haiku-4-5",
        "max_tokens": 200,
        "messages": [{ "role": "user", "content": prompt }]
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send().await
        .map_err(|e| format!("Claude API request failed: {e}"))?;

    let status = resp.status();
    let json: Value = resp.json().await
        .map_err(|e| format!("Claude API response parse failed: {e}"))?;

    if !status.is_success() {
        return Err(format!("Claude API returned HTTP {status}: {json}"));
    }

    let text = json.pointer("/content/0/text")
        .and_then(|v| v.as_str())
        .ok_or("unexpected Claude API response shape")?;

    // Strip any accidental markdown fences
    let clean = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();

    let parsed: Value = serde_json::from_str(clean)
        .map_err(|e| format!("Claude returned invalid JSON: {e}\nraw: {text}"))?;

    let hours = |k: &str| parsed.get(k).and_then(|v| v.as_f64());

    Ok(GameResult {
        steam_id,
        hltb_id: None,
        title: game_title.to_owned(),
        source: "claude_fallback".to_owned(),
        cached: false,
        main_story:    hours("main_story"),
        main_extra:    hours("main_extra"),
        completionist: hours("completionist"),
    })
}

// ─── HLTB game page fetch ────────────────────────────────────────────────────

async fn fetch_by_hltb_id(
    client: &Client,
    hltb_id: u64,
    steam_id: Option<u64>,
    cached: bool,
) -> Result<GameResult, String> {
    let url = format!("https://howlongtobeat.com/game/{hltb_id}");
    veprintln!("[hltb] fetching: {url}");

    let html = client.get(&url)
        .header("Referer", "https://howlongtobeat.com/")
        .send().await
        .map_err(|e| format!("game page fetch failed: {e}"))?
        .text().await
        .map_err(|e| format!("game page text failed: {e}"))?;

    let next_re = Regex::new(r#"<script id="__NEXT_DATA__"[^>]*>([\s\S]*?)</script>"#).unwrap();
    let json_str = next_re.captures(&html)
        .and_then(|c| c.get(1)).map(|m| m.as_str())
        .ok_or("could not find __NEXT_DATA__ in game page")?;

    let next: Value = serde_json::from_str(json_str)
        .map_err(|e| format!("__NEXT_DATA__ parse failed: {e}"))?;

    let game = next.pointer("/props/pageProps/game/data/game/0")
        .ok_or("unexpected __NEXT_DATA__ structure")?;

    let title = game.get("game_name")
        .and_then(|v| v.as_str())
        .unwrap_or("").to_owned();

    let (main_story, main_extra, completionist) = extract_times(game);

    Ok(GameResult {
        steam_id,
        hltb_id: Some(hltb_id),
        title,
        source: "hltb".to_owned(),
        cached,
        main_story,
        main_extra,
        completionist,
    })
}

// ─── Steam name lookup ────────────────────────────────────────────────────────

async fn steam_name(client: &Client, steam_id: u64) -> String {
    let url = format!("https://store.steampowered.com/api/appdetails?appids={steam_id}&filters=basic");
    match client.get(&url).send().await {
        Ok(r) => match r.json::<Value>().await {
            Ok(j) => j.pointer(&format!("/{steam_id}/data/name"))
                .and_then(|n| n.as_str()).unwrap_or("").to_owned(),
            Err(_) => String::new(),
        },
        Err(_) => String::new(),
    }
}

// ─── Steam title search ──────────────────────────────────────────────────────

/// Search Steam store by title and return the best matching App ID.
async fn steam_search_id(client: &Client, title: &str) -> Option<u64> {
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&cc=us&l=en",
        urlencoding::encode(title)
    );
    let json: Value = client.get(&url).send().await.ok()?.json().await.ok()?;
    let items = json.pointer("/items")?.as_array()?;

    // Find the closest name match
    let title_lower = title.to_lowercase();
    for item in items {
        let name = item.get("name")?.as_str()?;
        if name.to_lowercase() == title_lower {
            let id = item.get("id")?.as_u64()?;
            veprintln!("[hltb] found steam id {id} for '{title}'");
            return Some(id);
        }
    }
    // No exact match — return first result
    let id = items.first()?.get("id")?.as_u64()?;
    let name = items.first()?.get("name").and_then(|v| v.as_str()).unwrap_or("");
    veprintln!("[hltb] no exact steam match, using first result: '{name}' ({id})");
    Some(id)
}

// ─── Core lookup logic ────────────────────────────────────────────────────────

async fn lookup(
    client: &Client,
    conn: &Connection,
    config: &Config,
    steam_id: Option<u64>,
    title: Option<&str>,
    force: bool,
) -> Result<GameResult, String> {
    // Resolve game name and steam_id
    // If title provided, try to find a Steam ID for it so we can cache
    let mut resolved_steam_id = steam_id;
    let game_name = if let Some(t) = title {
        if resolved_steam_id.is_none() && !force {
            // Try to find Steam ID by title so we can use the cache
            if let Some(sid) = steam_search_id(client, t).await {
                // Check if already cached under this steam id
                if let Some(row) = cache_get(conn, sid) {
                    veprintln!("[hltb] cache hit via steam search: steam:{sid}");
                    return Ok(GameResult {
                        steam_id: Some(sid),
                        hltb_id: row.hltb_id,
                        title: row.title,
                        source: row.source,
                        cached: true,
                        main_story: row.main_story,
                        main_extra: row.main_extra,
                        completionist: row.completionist,
                    });
                }
                resolved_steam_id = Some(sid);
            }
        }
        t.to_owned()
    } else if let Some(sid) = steam_id {
        // Check cache first
        if !force {
            if let Some(row) = cache_get(conn, sid) {
                veprintln!("[hltb] cache hit steam:{sid}");
                return Ok(GameResult {
                    steam_id: Some(sid),
                    hltb_id: row.hltb_id,
                    title: row.title,
                    source: row.source,
                    cached: true,
                    main_story: row.main_story,
                    main_extra: row.main_extra,
                    completionist: row.completionist,
                });
            }
        }
        veprintln!("[hltb] cache miss steam:{sid}");
        let name = steam_name(client, sid).await;
        if name.is_empty() {
            return Err(format!("could not resolve Steam App ID {sid} to a game name"));
        }
        veprintln!("[hltb] steam name: {name}");
        name
    } else {
        return Err("provide --steam-id or --title".to_owned());
    };

    // Try HLTB via Brave
    let result = match bing_search_hltb_id(client, &config.search.api_key, &game_name).await {
        Ok(hltb_id) => {
            // Check if this hltb_id is already cached (catches misspelled title lookups)
            if !force {
                if let Some(row) = cache_get_by_hltb_id(conn, hltb_id) {
                    veprintln!("[hltb] cache hit via hltb_id:{hltb_id}");
                    // Try to find steam_id from cache row
                    let cached_steam_id = resolved_steam_id.or_else(|| {
                        conn.query_row(
                            "SELECT steam_id FROM games WHERE hltb_id = ?1 LIMIT 1",
                            params![hltb_id as i64],
                            |r| r.get::<_, i64>(0),
                        ).ok().map(|v| v as u64)
                    });
                    return Ok(GameResult {
                        steam_id: cached_steam_id,
                        hltb_id: row.hltb_id,
                        title: row.title,
                        source: row.source,
                        cached: true,
                        main_story: row.main_story,
                        main_extra: row.main_extra,
                        completionist: row.completionist,
                    });
                }
            }
            fetch_by_hltb_id(client, hltb_id, resolved_steam_id, false).await?
        }
        Err(e) => {
            veprintln!("[hltb] bing/hltb failed: {e}");

            // Claude fallback
            let claude_cfg = &config.claude;
            if claude_cfg.fallback_enabled {
                if let Some(claude_key) = &claude_cfg.api_key {
                    veprintln!("[hltb] trying claude fallback...");
                    let snippets = bing_search_snippets(client, &config.search.api_key, &game_name).await
                        .unwrap_or_default();
                    if snippets.is_empty() {
                        return Err(format!("no data found for \"{game_name}\" anywhere"));
                    }
                    claude_fallback(client, claude_key, &game_name, &snippets, resolved_steam_id).await?
                } else {
                    return Err(format!(
                        "no HLTB data found for \"{game_name}\" and no Claude API key configured for fallback"
                    ));
                }
            } else {
                return Err(format!("no HLTB data found for \"{game_name}\""));
            }
        }
    };

    // Cache by steam_id if we have one (includes steam id resolved from title search)
    // If we still don't have a steam_id, try searching Steam with the corrected title from HLTB
    if resolved_steam_id.is_none() {
        if let Some(sid) = steam_search_id(client, &result.title).await {
            resolved_steam_id = Some(sid);
        }
    }

    if let Some(sid) = resolved_steam_id {
        let mut r = result.clone();
        r.steam_id = Some(sid);
        if let Err(e) = cache_set(conn, sid, &r) {
            eprintln!("warning: {e}");
        } else {
            veprintln!("[hltb] cached steam:{sid}");
        }
        return Ok(r);
    }

    Ok(result)
}

// ─── Axum server ─────────────────────────────────────────────────────────────

// Per-request state — we open a fresh db connection and client each request.
// This tool is low-traffic so the overhead is negligible.
#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
}

#[derive(Deserialize)]
struct GameQuery {
    steam_id: Option<u64>,
    title: Option<String>,
    force: Option<bool>,
}

async fn handle_game(
    State(state): State<AppState>,
    Query(params): Query<GameQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let config = state.config.clone();
    let steam_id = params.steam_id;
    let title = params.title.clone();
    let force = params.force.unwrap_or(false);

    // Spawn on blocking thread since rusqlite is sync
    let result = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        let conn = open_db().map_err(|e| e)?;
        let client = make_client();
        rt.block_on(lookup(&client, &conn, &config, steam_id, title.as_deref(), force))
    }).await;

    match result {
        Ok(Ok(r))  => Ok(Json(serde_json::to_value(r).unwrap())),
        Ok(Err(e)) => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e })))),
        Err(e)     => Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() })))),
    }
}

async fn handle_health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "hltb" }))
}

// ─── main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let cfg_path = config_path(cli.config.as_ref());
    let config = match load_config(&cfg_path) {
        Ok(c) => c,
        Err(e) => { eprintln!("error: {e}"); process::exit(1); }
    };
    let conn = match open_db() {
        Ok(c) => c,
        Err(e) => { eprintln!("error: {e}"); process::exit(1); }
    };
    let client = make_client();

    if cli.serve {
        let port = config.server.port;
        VERBOSE.store(config.server.verbose, Ordering::Relaxed);
        let state = AppState {
            config: Arc::new(config),
        };
        let app = Router::new()
            .route("/game", get(handle_game))
            .route("/health", get(handle_health))
            .with_state(state);

        let addr = format!("0.0.0.0:{port}");
        veprintln!("[hltb] listening on http://{addr}");
        veprintln!("[hltb]   GET /game?steam_id=1145360");
        veprintln!("[hltb]   GET /game?title=Hades");
        veprintln!("[hltb]   GET /game?steam_id=1145360&force=true");
        veprintln!("[hltb]   GET /health");

        let listener = tokio::net::TcpListener::bind(&addr).await
            .unwrap_or_else(|e| { eprintln!("error: {e}"); process::exit(1); });
        axum::serve(listener, app).await
            .unwrap_or_else(|e| { eprintln!("error: {e}"); process::exit(1); });

    } else {
        let result = match lookup(
            &client, &conn, &config,
            cli.steam_id, cli.title.as_deref(), cli.force,
        ).await {
            Ok(r)  => r,
            Err(e) => { eprintln!("error: {e}"); process::exit(1); }
        };

        let out = serde_json::to_value(&result).unwrap();
        let json_str = if cli.pretty {
            serde_json::to_string_pretty(&out).unwrap()
        } else {
            serde_json::to_string(&out).unwrap()
        };
        println!("{json_str}");
    }
}
