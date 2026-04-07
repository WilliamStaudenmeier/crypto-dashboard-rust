use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone, Debug)]
struct ApiConfig {
    scheme: String,
    host: String,
    base_path: String,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    payload: Value,
    expires_at: Instant,
}

#[derive(Clone)]
struct AppState {
    client: Client,
    api_cfg: ApiConfig,
    api_cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    snapshot_lock: Arc<Mutex<()>>,
    snapshot_path: PathBuf,
    static_dir: PathBuf,
}

fn parse_api_base_url(url: &str) -> ApiConfig {
    let default_url = "https://api.coingecko.com/api/v3";
    let input = if url.is_empty() { default_url } else { url };

    let (scheme, after_scheme) = if let Some((s, rest)) = input.split_once("://") {
        (s.to_string(), rest)
    } else {
        ("https".to_string(), input)
    };

    let (host, base_path) = if let Some((h, path)) = after_scheme.split_once('/') {
        let resolved_host = if h.is_empty() {
            "api.coingecko.com".to_string()
        } else {
            h.to_string()
        };
        (resolved_host, format!("/{}", path))
    } else {
        let resolved_host = if after_scheme.is_empty() {
            "api.coingecko.com".to_string()
        } else {
            after_scheme.to_string()
        };
        (resolved_host, "".to_string())
    };

    ApiConfig {
        scheme,
        host,
        base_path,
    }
}

fn resolve_snapshot_path() -> PathBuf {
    if let Ok(path) = env::var("SNAPSHOT_PATH") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    PathBuf::from("db.json")
}

fn resolve_static_dir() -> PathBuf {
    if let Ok(path) = env::var("STATIC_DIR") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    PathBuf::from("static")
}

fn now_epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn is_valid_bootstrap_payload(payload: &Value) -> bool {
    payload.get("global").is_some()
        && payload.get("trending").is_some()
        && payload.get("markets").is_some()
}

async fn read_json_file(state: &AppState, file_path: &Path) -> Option<Value> {
    let _guard = state.snapshot_lock.lock().await;
    let bytes = tokio::fs::read(file_path).await.ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()
}

async fn write_json_file(state: &AppState, file_path: &Path, payload: &Value) -> bool {
    let _guard = state.snapshot_lock.lock().await;

    if let Some(parent) = file_path.parent() {
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return false;
        }
    }

    let temp_path = file_path.with_extension("tmp");
    let serialized = match serde_json::to_vec(payload) {
        Ok(v) => v,
        Err(_) => return false,
    };

    if tokio::fs::write(&temp_path, serialized).await.is_err() {
        return false;
    }

    if tokio::fs::rename(&temp_path, file_path).await.is_ok() {
        return true;
    }

    let _ = tokio::fs::remove_file(file_path).await;
    tokio::fs::rename(&temp_path, file_path).await.is_ok()
}

async fn fetch_json(state: &AppState, path: &str, cache_ttl_seconds: u64) -> Option<Value> {
    let full_path = format!("{}{}", state.api_cfg.base_path, path);

    if cache_ttl_seconds > 0 {
        let cache = state.api_cache.lock().await;
        if let Some(entry) = cache.get(&full_path) {
            if Instant::now() < entry.expires_at {
                return Some(entry.payload.clone());
            }
        }
    }

    let url = format!("{}://{}{}", state.api_cfg.scheme, state.api_cfg.host, full_path);
    let response = state.client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let parsed: Value = response.json().await.ok()?;

    if cache_ttl_seconds > 0 {
        let mut cache = state.api_cache.lock().await;
        cache.insert(
            full_path,
            CacheEntry {
                payload: parsed.clone(),
                expires_at: Instant::now() + Duration::from_secs(cache_ttl_seconds),
            },
        );
    }

    Some(parsed)
}

async fn fetch_bootstrap_live(state: &AppState) -> Option<Value> {
    let markets_path = "/coins/markets?vs_currency=usd&order=market_cap_desc&sparkline=false&price_change_percentage=24h&per_page=20&page=1";

    let global_fut = fetch_json(state, "/global", 60);
    let trending_fut = fetch_json(state, "/search/trending", 60);
    let markets_fut = fetch_json(state, markets_path, 30);

    let (global, trending, markets) = tokio::join!(global_fut, trending_fut, markets_fut);

    Some(json!({
        "global": global?,
        "trending": trending?,
        "markets": markets?,
        "meta": {
            "source": "live",
            "updated_at_epoch_ms": now_epoch_ms()
        }
    }))
}

fn json_with_status(status: StatusCode, payload: Value) -> Response {
    (status, Json(payload)).into_response()
}

fn json_no_cache(status: StatusCode, payload: Value) -> Response {
    let mut response = (status, Json(payload)).into_response();
    response.headers_mut().insert(
        "Cache-Control",
        HeaderValue::from_static("no-cache"),
    );
    response
}

async fn health_handler() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn global_handler(State(state): State<AppState>) -> Response {
    match fetch_json(&state, "/global", 60).await {
        Some(data) => json_with_status(StatusCode::OK, data),
        None => json_with_status(
            StatusCode::BAD_GATEWAY,
            json!({ "error": "Failed to fetch global market data" }),
        ),
    }
}

async fn trending_handler(State(state): State<AppState>) -> Response {
    match fetch_json(&state, "/search/trending", 60).await {
        Some(data) => json_with_status(StatusCode::OK, data),
        None => json_with_status(
            StatusCode::BAD_GATEWAY,
            json!({ "error": "Failed to fetch trending data" }),
        ),
    }
}

async fn markets_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let vs_currency = params.get("vs_currency").map_or("usd", String::as_str);
    let page = params.get("page").map_or("1", String::as_str);
    let per_page = params.get("per_page").map_or("20", String::as_str);

    let path = format!(
        "/coins/markets?vs_currency={vs_currency}&order=market_cap_desc&sparkline=false&price_change_percentage=24h&per_page={per_page}&page={page}"
    );

    match fetch_json(&state, &path, 30).await {
        Some(data) => json_with_status(StatusCode::OK, data),
        None => json_with_status(
            StatusCode::BAD_GATEWAY,
            json!({ "error": "Failed to fetch market list" }),
        ),
    }
}

async fn history_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(coin_id) = params.get("coin_id") else {
        return json_with_status(
            StatusCode::BAD_REQUEST,
            json!({ "error": "coin_id is required" }),
        );
    };

    let days = params.get("days").map_or("365", String::as_str);
    let vs_currency = params.get("vs_currency").map_or("usd", String::as_str);

    let path = format!(
        "/coins/{coin_id}/market_chart?vs_currency={vs_currency}&days={days}&interval=daily"
    );

    match fetch_json(&state, &path, 300).await {
        Some(data) => json_with_status(StatusCode::OK, data),
        None => json_with_status(
            StatusCode::BAD_GATEWAY,
            json!({ "error": "Failed to fetch market history" }),
        ),
    }
}

async fn bootstrap_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let force_refresh = params
        .get("refresh")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !force_refresh {
        if let Some(snapshot) = read_json_file(&state, &state.snapshot_path).await {
            if is_valid_bootstrap_payload(&snapshot) {
                let mut stale_payload = snapshot;
                if !stale_payload.get("meta").is_some_and(Value::is_object) {
                    stale_payload["meta"] = json!({});
                }
                stale_payload["meta"]["source"] = json!("snapshot");
                stale_payload["meta"]["served_at_epoch_ms"] = json!(now_epoch_ms());
                return json_no_cache(StatusCode::OK, stale_payload);
            }
        }
    }

    if let Some(live_payload) = fetch_bootstrap_live(&state).await {
        let _ = write_json_file(&state, &state.snapshot_path, &live_payload).await;
        return json_no_cache(StatusCode::OK, live_payload);
    }

    if let Some(fallback) = read_json_file(&state, &state.snapshot_path).await {
        if is_valid_bootstrap_payload(&fallback) {
            let mut fallback_payload = fallback;
            if !fallback_payload.get("meta").is_some_and(Value::is_object) {
                fallback_payload["meta"] = json!({});
            }
            fallback_payload["meta"]["source"] = json!("snapshot-fallback");
            fallback_payload["meta"]["served_at_epoch_ms"] = json!(now_epoch_ms());
            fallback_payload["meta"]["warning"] = json!("live-refresh-failed");
            return json_no_cache(StatusCode::OK, fallback_payload);
        }
    }

    json_with_status(
        StatusCode::BAD_GATEWAY,
        json!({ "error": "Failed to fetch bootstrap market data" }),
    )
}

async fn serve_file(path: PathBuf, content_type: &'static str, cache_control: &'static str) -> Response {
    match tokio::fs::read_to_string(path).await {
        Ok(body) => {
            let mut response = (StatusCode::OK, body).into_response();
            response.headers_mut().insert(
                "Content-Type",
                HeaderValue::from_static(content_type),
            );
            response.headers_mut().insert(
                "Cache-Control",
                HeaderValue::from_static(cache_control),
            );
            response
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn index_handler(State(state): State<AppState>) -> Response {
    let path = state.static_dir.join("index.html");
    serve_file(path, "text/html; charset=utf-8", "no-cache").await
}

async fn styles_handler(State(state): State<AppState>) -> Response {
    let path = state.static_dir.join("styles.css");
    serve_file(path, "text/css; charset=utf-8", "public, max-age=300").await
}

async fn app_js_handler(State(state): State<AppState>) -> Response {
    let path = state.static_dir.join("app.js");
    serve_file(
        path,
        "application/javascript; charset=utf-8",
        "public, max-age=300",
    )
    .await
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    let port = env::var("PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8080);

    let base_url = env::var("COINGECKO_BASE_URL").unwrap_or_default();
    let api_cfg = parse_api_base_url(&base_url);

    let mut default_headers = reqwest::header::HeaderMap::new();
    if let Ok(api_key) = env::var("COINGECKO_API_KEY") {
        if !api_key.is_empty() {
            if let Ok(header_value) = HeaderValue::from_str(&api_key) {
                default_headers.insert("x-cg-demo-api-key", header_value);
            }
        }
    }

    let client = Client::builder()
        .default_headers(default_headers)
        .build()
        .expect("failed to build reqwest client");

    let state = AppState {
        client,
        api_cfg,
        api_cache: Arc::new(Mutex::new(HashMap::new())),
        snapshot_lock: Arc::new(Mutex::new(())),
        snapshot_path: resolve_snapshot_path(),
        static_dir: resolve_static_dir(),
    };

    let cors_layer = if let Ok(origin) = env::var("FRONTEND_ORIGIN") {
        if origin.is_empty() || origin == "*" {
            CorsLayer::new().allow_origin(Any)
        } else if let Ok(header) = HeaderValue::from_str(&origin) {
            CorsLayer::new().allow_origin(header)
        } else {
            CorsLayer::new().allow_origin(Any)
        }
    } else {
        CorsLayer::new().allow_origin(Any)
    }
    .allow_methods([http::Method::GET])
    .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/global", get(global_handler))
        .route("/api/bootstrap", get(bootstrap_handler))
        .route("/api/trending", get(trending_handler))
        .route("/api/markets", get(markets_handler))
        .route("/api/history", get(history_handler))
        .route("/", get(index_handler))
        .route("/styles.css", get(styles_handler))
        .route("/app.js", get(app_js_handler))
        .with_state(state)
        .layer(cors_layer);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("Crypto Dashboard Rust listening on port {port}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");

    axum::serve(listener, app)
        .await
        .expect("failed to start server");
}

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parse_api_base_url_uses_default_when_empty() {
            let cfg = parse_api_base_url("");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "api.coingecko.com");
            assert_eq!(cfg.base_path, "/api/v3");
        }

        #[test]
        fn parse_api_base_url_supports_custom_https_path() {
            let cfg = parse_api_base_url("https://example.com/custom/base");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "example.com");
            assert_eq!(cfg.base_path, "/custom/base");
        }

        #[test]
        fn parse_api_base_url_supports_host_without_path() {
            let cfg = parse_api_base_url("https://api.example.com");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "api.example.com");
            assert_eq!(cfg.base_path, "");
        }

        #[test]
        fn parse_api_base_url_supports_http_scheme() {
            let cfg = parse_api_base_url("http://localhost:8080/v1");
            assert_eq!(cfg.scheme, "http");
            assert_eq!(cfg.host, "localhost:8080");
            assert_eq!(cfg.base_path, "/v1");
        }

        #[test]
        fn parse_api_base_url_falls_back_to_default_host_when_empty_host() {
            let cfg = parse_api_base_url("https:///v3");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "api.coingecko.com");
            assert_eq!(cfg.base_path, "/v3");
        }

        #[test]
        fn parse_api_base_url_handles_host_only_input_without_scheme() {
            let cfg = parse_api_base_url("coingecko.example.com");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "coingecko.example.com");
            assert_eq!(cfg.base_path, "");
        }

        #[test]
        fn parse_api_base_url_handles_path_with_query() {
            let cfg = parse_api_base_url("https://example.com/api/v3?x=1");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "example.com");
            assert_eq!(cfg.base_path, "/api/v3?x=1");
        }

        #[test]
        fn parse_api_base_url_handles_trailing_slash() {
            let cfg = parse_api_base_url("https://example.com/");
            assert_eq!(cfg.scheme, "https");
            assert_eq!(cfg.host, "example.com");
            assert_eq!(cfg.base_path, "/");
        }

        #[test]
        fn bootstrap_payload_validation_requires_all_sections() {
            let missing_all = json!({});
            assert!(!is_valid_bootstrap_payload(&missing_all));

            let only_global = json!({ "global": {} });
            assert!(!is_valid_bootstrap_payload(&only_global));

            let missing_markets = json!({
                "global": {},
                "trending": {}
            });
            assert!(!is_valid_bootstrap_payload(&missing_markets));

            let full_payload = json!({
                "global": { "data": {} },
                "trending": { "coins": [] },
                "markets": []
            });
            assert!(is_valid_bootstrap_payload(&full_payload));
        }

        #[test]
        fn now_epoch_ms_returns_positive_timestamp() {
            let ts = now_epoch_ms();
            assert!(ts > 0);
        }

        #[tokio::test]
        async fn write_and_read_json_file_round_trip() {
            let temp_dir = std::env::temp_dir().join(format!("crypto-dashboard-rust-test-{}", now_epoch_ms()));
            let snapshot_path = temp_dir.join("db.json");

            let state = AppState {
                client: Client::builder().build().expect("client"),
                api_cfg: parse_api_base_url(""),
                api_cache: Arc::new(Mutex::new(HashMap::new())),
                snapshot_lock: Arc::new(Mutex::new(())),
                snapshot_path: snapshot_path.clone(),
                static_dir: temp_dir.clone(),
            };

            let payload = json!({
                "global": { "data": { "total_market_cap": { "usd": 1 } } },
                "trending": { "coins": [] },
                "markets": []
            });

            let wrote = write_json_file(&state, &snapshot_path, &payload).await;
            assert!(wrote);

            let loaded = read_json_file(&state, &snapshot_path).await;
            assert_eq!(loaded, Some(payload));

            let _ = tokio::fs::remove_file(snapshot_path).await;
            let _ = tokio::fs::remove_dir_all(temp_dir).await;
        }

        #[tokio::test]
        async fn read_json_file_returns_none_for_invalid_json() {
            let temp_dir = std::env::temp_dir().join(format!("crypto-dashboard-rust-test-invalid-{}", now_epoch_ms()));
            let snapshot_path = temp_dir.join("db.json");
            tokio::fs::create_dir_all(&temp_dir).await.expect("mkdir");
            tokio::fs::write(&snapshot_path, b"not-json").await.expect("write");

            let state = AppState {
                client: Client::builder().build().expect("client"),
                api_cfg: parse_api_base_url(""),
                api_cache: Arc::new(Mutex::new(HashMap::new())),
                snapshot_lock: Arc::new(Mutex::new(())),
                snapshot_path: snapshot_path.clone(),
                static_dir: temp_dir.clone(),
            };

            let loaded = read_json_file(&state, &snapshot_path).await;
            assert!(loaded.is_none());

            let _ = tokio::fs::remove_file(snapshot_path).await;
            let _ = tokio::fs::remove_dir_all(temp_dir).await;
        }
    }
