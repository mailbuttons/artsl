//! The service mailbuttons.com runs behind `/art/`:
//!
//!   GET    /art/latest.json        picture of the day; public; ETag + 304
//!   GET    /art/archive/{id}.json  an earlier picture
//!   GET    /art/healthz            JSON; 503 when the picture of the day is stale
//!   POST   /art/v1/subscribe       Bearer mbk_ key once -> an art-only token
//!   DELETE /art/v1/subscribe       Bearer art_ token -> revoked
//!   POST   /art/v1/generate        Bearer art_ token -> draw one now; daily quota
//!   GET    /art/v1/admin/stats     Bearer admin token (optional feature)
//!   POST   /art/v1/admin/revoke    Bearer admin token; {"hash_prefix": "..."}
//!
//! A background task redraws the picture of the day on a schedule, purges
//! old usage rows and trims the archive. State lives in one redb file.

use crate::art::{one_line, Art};
use crate::builtin;
use crate::generate::Generator;
use crate::news;
use crate::store::{Store, TokenRecord};
use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use rand::{distr::Alphanumeric, Rng};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tower_http::{timeout::TimeoutLayer, trace::TraceLayer};
use tracing::{error, info, warn};

pub struct ServeConfig {
    pub listen: String,
    pub data_dir: PathBuf,
    pub mailbuttons_url: String,
    pub feeds: Vec<String>,
    pub headlines: usize,
    pub refresh: Duration,
    pub daily_quota: u32,
    pub concurrency: usize,
    pub archive_days: u64,
    pub admin_token: Option<String>,
}

pub struct AppState {
    cfg: ServeConfig,
    gen: Generator,
    http: reqwest::Client,
    store: Store,
    drawing: Semaphore,
    upstream: Semaphore,
}

const USAGE_KEEP_DAYS: i64 = 7;
const MAX_BODY: usize = 4096;

pub fn latest_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("latest.json")
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

pub fn hash(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|k| !k.is_empty() && k.len() < 256)
        .map(str::to_owned)
}

fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": code, "message": message.into() })),
    )
        .into_response()
}

/// Draw from the current headlines and write `archive/{id}.json`; if
/// `as_latest`, also swap it in as `latest.json` atomically.
pub async fn publish_once(
    gen: &Generator,
    http: &reqwest::Client,
    feeds: &[String],
    headlines: usize,
    data_dir: &std::path::Path,
    theme: Option<&str>,
    as_latest: bool,
) -> Result<Art> {
    let items = news::fetch_headlines(http, feeds, headlines).await?;
    let art = gen.draw(&items, theme).await?;
    let archive = data_dir.join("archive");
    tokio::fs::create_dir_all(&archive).await?;
    let bytes = serde_json::to_vec_pretty(&art)?;
    tokio::fs::write(archive.join(format!("{}.json", art.id)), &bytes).await?;
    if as_latest {
        let latest = latest_path(data_dir);
        let tmp = data_dir.join("latest.json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &latest).await?;
    }
    Ok(art)
}

pub async fn run(cfg: ServeConfig, gen: Generator) -> Result<()> {
    tokio::fs::create_dir_all(&cfg.data_dir)
        .await
        .with_context(|| format!("creating {}", cfg.data_dir.display()))?;
    let store = Store::open(&cfg.data_dir.join("artsl.redb"))?;
    let http = reqwest::Client::builder()
        .user_agent(concat!("artsl-server/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let listen = cfg.listen.clone();
    let admin_enabled = cfg.admin_token.is_some();
    let state = Arc::new(AppState {
        drawing: Semaphore::new(cfg.concurrency.max(1)),
        upstream: Semaphore::new(4),
        cfg,
        gen,
        http,
        store,
    });

    tokio::spawn(housekeeping(state.clone()));

    let quick: Router<Arc<AppState>> = Router::new()
        .route("/art/latest.json", get(latest))
        .route("/art/archive/{id}", get(archive))
        .route("/art/healthz", get(healthz))
        .route("/art/v1/subscribe", post(subscribe).delete(unsubscribe))
        .route("/art/v1/admin/stats", get(admin_stats))
        .route("/art/v1/admin/revoke", post(admin_revoke))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ));
    let slow: Router<Arc<AppState>> = Router::new().route("/art/v1/generate", post(generate_now));
    let app = quick
        .merge(slow)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    info!(%listen, admin_enabled, "artsl art service listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("shutting down");
        })
        .await?;
    Ok(())
}

async fn latest_age(data_dir: &std::path::Path) -> Option<Duration> {
    tokio::fs::metadata(latest_path(data_dir))
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
}

/// Redraw the picture of the day when it is due, purge usage rows older than
/// a week, and trim the archive. Checks once a minute; cheap when idle.
async fn housekeeping(state: Arc<AppState>) {
    let mut last_maintenance = Instant::now() - Duration::from_secs(86_400);
    loop {
        let age = latest_age(&state.cfg.data_dir)
            .await
            .unwrap_or(Duration::MAX);
        if age >= state.cfg.refresh {
            let _permit = state.drawing.acquire().await;
            match publish_once(
                &state.gen,
                &state.http,
                &state.cfg.feeds,
                state.cfg.headlines,
                &state.cfg.data_dir,
                None,
                true,
            )
            .await
            {
                Ok(art) => {
                    info!(id = %art.id, title = %art.title, source = %art.source, "picture of the day redrawn")
                }
                Err(e) => error!(error = %format!("{e:#}"), "picture of the day refresh failed"),
            }
        }
        if last_maintenance.elapsed() >= Duration::from_secs(86_400) {
            last_maintenance = Instant::now();
            let keep_from = (Utc::now() - chrono::Duration::days(USAGE_KEEP_DAYS))
                .format("%Y-%m-%d")
                .to_string();
            match state.store.purge_usage_before(&keep_from) {
                Ok(n) if n > 0 => info!(rows = n, "purged old usage rows"),
                Ok(_) => {}
                Err(e) => warn!(error = %e, "usage purge failed"),
            }
            match trim_archive(&state.cfg.data_dir, state.cfg.archive_days).await {
                Ok(n) if n > 0 => info!(files = n, "trimmed archive"),
                Ok(_) => {}
                Err(e) => warn!(error = %e, "archive trim failed"),
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn trim_archive(data_dir: &std::path::Path, keep_days: u64) -> Result<usize> {
    if keep_days == 0 {
        return Ok(0);
    }
    let dir = data_dir.join("archive");
    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(_) => return Ok(0),
    };
    let cutoff = Duration::from_secs(keep_days * 86_400);
    let mut removed = 0;
    while let Some(entry) = rd.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let old = entry
            .metadata()
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > cutoff)
            .unwrap_or(false);
        if old && tokio::fs::remove_file(&path).await.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn etag_for(bytes: &[u8]) -> String {
    format!("\"{}\"", &hex::encode(Sha256::digest(bytes))[..16])
}

async fn latest(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match tokio::fs::read(latest_path(&state.cfg.data_dir)).await {
        Ok(bytes) => {
            let etag = etag_for(&bytes);
            let matches = headers
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(',').any(|t| t.trim() == etag))
                .unwrap_or(false);
            let common = [
                (header::ETAG, etag.clone()),
                (header::CACHE_CONTROL, "public, max-age=300".to_string()),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string()),
            ];
            if matches {
                (StatusCode::NOT_MODIFIED, common).into_response()
            } else {
                (common, [(header::CONTENT_TYPE, "application/json")], bytes).into_response()
            }
        }
        Err(_) => (
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ],
            Json(builtin::builtin()),
        )
            .into_response(),
    }
}

async fn archive(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let id = id.strip_suffix(".json").unwrap_or(&id).to_string();
    let safe = !id.is_empty()
        && id.len() < 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return api_error(StatusCode::NOT_FOUND, "not_found", "no such picture");
    }
    let path = state
        .cfg
        .data_dir
        .join("archive")
        .join(format!("{id}.json"));
    match tokio::fs::read(path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => api_error(StatusCode::NOT_FOUND, "not_found", "no such picture"),
    }
}

/// Health for monitoring: 200 while the picture of the day is younger than
/// three refresh periods, 503 otherwise (including before the first draw).
async fn healthz(State(state): State<Arc<AppState>>) -> Response {
    let age = latest_age(&state.cfg.data_dir).await;
    let stale_after = state.cfg.refresh * 3;
    let (status, label) = match age {
        Some(a) if a < stale_after => (StatusCode::OK, "ok"),
        Some(_) => (StatusCode::SERVICE_UNAVAILABLE, "stale"),
        None => (StatusCode::SERVICE_UNAVAILABLE, "no_picture_yet"),
    };
    let latest_id = tokio::fs::read(latest_path(&state.cfg.data_dir))
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["id"].as_str().map(str::to_owned));
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "status": label,
            "latest_id": latest_id,
            "latest_age_secs": age.map(|a| a.as_secs()),
            "refresh_secs": state.cfg.refresh.as_secs(),
            "drawing_slots_free": state.drawing.available_permits(),
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
        .into_response()
}

/// Does mailbuttons accept this account key? One authenticated call.
async fn mailbuttons_accepts(state: &AppState, key: &str) -> Result<bool> {
    let resp = state
        .http
        .get(format!(
            "{}/api/v1/mailboxes",
            state.cfg.mailbuttons_url.trim_end_matches('/')
        ))
        .bearer_auth(key)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("asking mailbuttons about the key")?;
    match resp.status().as_u16() {
        200 => Ok(true),
        401 | 403 => Ok(false),
        other => anyhow::bail!("mailbuttons answered {other}"),
    }
}

fn mint_token() -> String {
    let tail: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(40)
        .map(char::from)
        .collect();
    format!("art_{tail}")
}

async fn subscribe(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = match bearer(&headers) {
        Some(k) if k.starts_with("mbk_") => k,
        Some(_) => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "wrong_credential",
                "subscribe takes your mailbuttons account key (mbk_...), used once",
            )
        }
        None => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "missing_key",
                "send your mailbuttons API key as a Bearer token",
            )
        }
    };
    let _permit =
        match tokio::time::timeout(Duration::from_secs(20), state.upstream.acquire()).await {
            Ok(Ok(p)) => p,
            _ => {
                return api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "busy",
                    "try again in a moment",
                )
            }
        };
    match mailbuttons_accepts(&state, &key).await {
        Ok(true) => {}
        Ok(false) => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "invalid_key",
                "mailbuttons did not accept that key",
            )
        }
        Err(e) => {
            warn!(error = %format!("{e:#}"), "key check failed");
            return api_error(
                StatusCode::BAD_GATEWAY,
                "upstream",
                "could not check the key with mailbuttons",
            );
        }
    }
    let label = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["label"].as_str().map(|s| one_line(s, 60)))
        .unwrap_or_default();
    let token = mint_token();
    let rec = TokenRecord {
        created_at: now_rfc3339(),
        last_used_at: None,
        label: label.clone(),
        revoked: false,
        pictures: 0,
    };
    let h = hash(&token);
    if let Err(e) = state.store.insert_token(&h, &rec) {
        error!(error = %e, "could not store token");
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store",
            "could not store the subscription",
        );
    }
    info!(token = %&h[..12], %label, "new art token");
    (
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "token": token,
            "created_at": rec.created_at,
            "label": label,
            "quota_per_day": state.cfg.daily_quota,
        })),
    )
        .into_response()
}

async fn unsubscribe(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let token = match bearer(&headers) {
        Some(t) if t.starts_with("art_") => t,
        _ => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "missing_token",
                "send your art token as a Bearer token",
            )
        }
    };
    match state.store.revoke(&hash(&token)) {
        Ok(true) => {
            info!(token = %&hash(&token)[..12], "token revoked by owner");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "that token is not active",
        ),
        Err(e) => {
            error!(error = %e, "revoke failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store",
                "could not revoke",
            )
        }
    }
}

async fn generate_now(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let token = match bearer(&headers) {
        Some(t) if t.starts_with("art_") => t,
        Some(t) if t.starts_with("mbk_") => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "subscribe_first",
                "drawing takes an art token, not an account key; run `artsl subscribe`",
            )
        }
        _ => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "missing_token",
                "send your art token as a Bearer token",
            )
        }
    };
    let h = hash(&token);
    match state.store.get_token(&h) {
        Ok(Some(rec)) if !rec.revoked => {}
        Ok(_) => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "that token is not active; run `artsl subscribe`",
            )
        }
        Err(e) => {
            error!(error = %e, "token lookup failed");
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "store",
                "could not check the token",
            );
        }
    }
    match state.store.take_quota(&h, &today(), state.cfg.daily_quota) {
        Ok(Ok(_)) => {}
        Ok(Err(limit)) => {
            return api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "quota",
                format!("today's allowance of {limit} fresh pictures is used up; the picture of the day is still free"),
            )
        }
        Err(e) => {
            error!(error = %e, "quota failed");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "store", "could not count the picture");
        }
    }
    let theme = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["theme"].as_str().map(|s| one_line(s, 200)))
        .filter(|s| !s.is_empty());

    let permit = match tokio::time::timeout(Duration::from_secs(120), state.drawing.acquire()).await
    {
        Ok(Ok(p)) => p,
        _ => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "busy",
                "the studio is busy; try again in a minute",
            )
        }
    };
    let started = Instant::now();
    let result = publish_once(
        &state.gen,
        &state.http,
        &state.cfg.feeds,
        state.cfg.headlines,
        &state.cfg.data_dir,
        theme.as_deref(),
        false,
    )
    .await;
    drop(permit);
    match result {
        Ok(art) => {
            let _ = state.store.note_picture(&h, &now_rfc3339());
            info!(token = %&h[..12], id = %art.id, secs = started.elapsed().as_secs(), "picture drawn on demand");
            ([(header::CACHE_CONTROL, "no-store")], Json(art)).into_response()
        }
        Err(e) => {
            warn!(token = %&h[..12], error = %format!("{e:#}"), "on-demand draw failed");
            api_error(StatusCode::BAD_GATEWAY, "draw_failed", format!("{e:#}"))
        }
    }
}

fn admin_ok(state: &AppState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let Some(expected) = state.cfg.admin_token.as_deref() else {
        return Err(Box::new(api_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "not found",
        )));
    };
    match bearer(headers) {
        // Compared as digests so the comparison takes the same time whatever
        // the input.
        Some(t) if hash(&t) == hash(expected) => Ok(()),
        _ => Err(Box::new(api_error(
            StatusCode::UNAUTHORIZED,
            "admin",
            "admin token required",
        ))),
    }
}

async fn admin_stats(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(r) = admin_ok(&state, &headers) {
        return *r;
    }
    let stats = match state.store.stats(&today()) {
        Ok(s) => s,
        Err(e) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "store", e.to_string()),
    };
    let tokens = state.store.list_tokens(50).unwrap_or_default();
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "stats": stats,
            "recent_tokens": tokens,
            "latest_age_secs": latest_age(&state.cfg.data_dir).await.map(|a| a.as_secs()),
            "daily_quota": state.cfg.daily_quota,
        })),
    )
        .into_response()
}

async fn admin_revoke(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(r) = admin_ok(&state, &headers) {
        return *r;
    }
    let prefix = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v["hash_prefix"].as_str().map(str::to_owned))
        .unwrap_or_default();
    match state.store.revoke_by_prefix(&prefix) {
        Ok(n) => {
            info!(%prefix, revoked = n, "admin revoke");
            Json(json!({ "revoked": n })).into_response()
        }
        Err(e) => api_error(StatusCode::BAD_REQUEST, "bad_prefix", e.to_string()),
    }
}
