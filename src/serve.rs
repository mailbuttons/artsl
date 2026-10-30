//! The service mailbuttons.com runs behind `/art/`:
//!
//!   GET  /art/latest.json        the picture of the day, public, cached
//!   GET  /art/archive/{id}.json  an earlier picture
//!   GET  /art/healthz
//!   POST /art/v1/generate        draw one now; Bearer <mbk_ key>; daily quota
//!
//! A background task redraws the picture of the day on a schedule.
//! `artsl publish` does the same redraw once, for a systemd timer.

use crate::art::Art;
use crate::builtin;
use crate::generate::Generator;
use crate::news;
use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

pub struct ServeConfig {
    pub listen: String,
    pub data_dir: PathBuf,
    pub mailbuttons_url: String,
    pub feeds: Vec<String>,
    pub headlines: usize,
    pub refresh: Duration,
    pub daily_quota: u32,
    pub concurrency: usize,
}

pub struct AppState {
    cfg: ServeConfig,
    gen: Generator,
    http: reqwest::Client,
    valid_keys: Mutex<HashMap<String, Instant>>,
    usage: Mutex<HashMap<String, (String, u32)>>,
    drawing: Semaphore,
}

const KEY_CACHE_TTL: Duration = Duration::from_secs(600);

pub fn latest_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("latest.json")
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
    let http = reqwest::Client::builder()
        .user_agent(concat!("artsl-server/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let listen = cfg.listen.clone();
    let state = Arc::new(AppState {
        drawing: Semaphore::new(cfg.concurrency.max(1)),
        cfg,
        gen,
        http,
        valid_keys: Mutex::new(HashMap::new()),
        usage: Mutex::new(HashMap::new()),
    });

    tokio::spawn(refresher(state.clone()));

    let app = Router::new()
        .route("/art/latest.json", get(latest))
        .route("/art/archive/{id}", get(archive))
        .route("/art/healthz", get(|| async { "ok" }))
        .route("/art/v1/generate", post(generate_now))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    eprintln!("artsl: serving on http://{listen}/art/");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn refresher(state: Arc<AppState>) {
    loop {
        let latest = latest_path(&state.cfg.data_dir);
        let age = tokio::fs::metadata(&latest)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
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
                Ok(art) => eprintln!("artsl: picture of the day is now {} ({})", art.id, art.title),
                Err(e) => eprintln!("artsl: refresh failed: {e:#}"),
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn api_error(status: StatusCode, code: &str, message: String) -> Response {
    (status, Json(json!({ "error": code, "message": message }))).into_response()
}

async fn latest(State(state): State<Arc<AppState>>) -> Response {
    match tokio::fs::read(latest_path(&state.cfg.data_dir)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "public, max-age=300"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ],
            bytes,
        )
            .into_response(),
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
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return api_error(StatusCode::NOT_FOUND, "not_found", "no such picture".into());
    }
    let path = state.cfg.data_dir.join("archive").join(format!("{id}.json"));
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
        Err(_) => api_error(StatusCode::NOT_FOUND, "not_found", "no such picture".into()),
    }
}

fn key_hash(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// A key is good if mailbuttons answers an authenticated call with it. The
/// answer is remembered for ten minutes so a subscriber's taps don't hammer
/// the platform.
async fn key_is_valid(state: &AppState, key: &str) -> Result<bool> {
    let h = key_hash(key);
    if let Some(seen) = state.valid_keys.lock().unwrap().get(&h) {
        if seen.elapsed() < KEY_CACHE_TTL {
            return Ok(true);
        }
    }
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
        200 => {
            state.valid_keys.lock().unwrap().insert(h, Instant::now());
            Ok(true)
        }
        401 | 403 => Ok(false),
        other => anyhow::bail!("mailbuttons answered {other}"),
    }
}

fn take_quota(state: &AppState, key: &str) -> Result<(), u32> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut usage = state.usage.lock().unwrap();
    let entry = usage.entry(key_hash(key)).or_insert_with(|| (today.clone(), 0));
    if entry.0 != today {
        *entry = (today, 0);
    }
    if entry.1 >= state.cfg.daily_quota {
        return Err(state.cfg.daily_quota);
    }
    entry.1 += 1;
    Ok(())
}

async fn generate_now(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let key = match key {
        Some(k) => k.to_string(),
        None => {
            return api_error(
                StatusCode::UNAUTHORIZED,
                "missing_key",
                "send your mailbuttons API key as a Bearer token".into(),
            )
        }
    };
    match key_is_valid(&state, &key).await {
        Ok(true) => {}
        Ok(false) => {
            return api_error(StatusCode::UNAUTHORIZED, "invalid_key", "that key is not active".into())
        }
        Err(e) => {
            return api_error(
                StatusCode::BAD_GATEWAY,
                "upstream",
                format!("could not check the key: {e:#}"),
            )
        }
    }
    if let Err(limit) = take_quota(&state, &key) {
        return api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "quota",
            format!("today's allowance of {limit} fresh pictures is used up; the picture of the day is still free"),
        );
    }
    let theme = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v["theme"].as_str().map(|s| s.chars().take(200).collect::<String>()))
    };

    let permit = match tokio::time::timeout(Duration::from_secs(120), state.drawing.acquire()).await {
        Ok(Ok(p)) => p,
        _ => {
            return api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "busy",
                "the studio is busy; try again in a minute".into(),
            )
        }
    };
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
        Ok(art) => ([(header::CACHE_CONTROL, "no-store")], Json(art)).into_response(),
        Err(e) => api_error(StatusCode::BAD_GATEWAY, "draw_failed", format!("{e:#}")),
    }
}
