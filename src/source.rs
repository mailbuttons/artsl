//! Which picture to show. In order of preference, skipping what isn't
//! available: a file you named, the built-in, a fresh one from your
//! subscription, the cached feed copy if young enough, the public feed,
//! the cache however old, and finally the built-in again.

use crate::art::Art;
use crate::builtin;
use crate::config::{cache_dir, Config};
use crate::mailbuttons::{self, Mailbuttons};
use anyhow::{Context, Result};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    Cache,
    Feed,
    Fresh,
    File,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Builtin => "built in",
            Origin::Cache => "picture of the day (cached)",
            Origin::Feed => "picture of the day",
            Origin::Fresh => "drawn just now for you",
            Origin::File => "from file",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct PickOpts {
    pub builtin: bool,
    pub fresh: bool,
    pub offline: bool,
    pub theme: Option<String>,
    pub file: Option<PathBuf>,
    pub mailbuttons_url: Option<String>,
}

pub fn cache_path() -> PathBuf {
    cache_dir().join("latest.json")
}

pub fn load_cache() -> Option<(Art, Duration)> {
    let path = cache_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .unwrap_or(Duration::MAX);
    Art::from_json(&text).ok().map(|a| (a, age))
}

pub fn save_cache(art: &Art) -> Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(art)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub async fn pick(cfg: &Config, opts: &PickOpts) -> Result<(Art, Origin)> {
    if let Some(path) = &opts.file {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        return Ok((Art::from_json(&text)?, Origin::File));
    }
    if opts.builtin {
        return Ok((builtin::builtin(), Origin::Builtin));
    }

    if opts.fresh {
        match cfg.mailbuttons.api_key.as_deref().filter(|k| !k.is_empty()) {
            None => {
                eprintln!("artsl: --fresh needs a subscription. Run `artsl subscribe` first. Showing the picture of the day instead.");
            }
            Some(key) if opts.offline => {
                let _ = key;
                eprintln!("artsl: --fresh and --offline don't mix; showing what's cached.");
            }
            Some(key) => {
                let base = opts
                    .mailbuttons_url
                    .clone()
                    .unwrap_or_else(|| cfg.mailbuttons.base_url.clone());
                let mb = Mailbuttons::new(&base)?;
                let spinner = Spinner::start("Reading the news and drawing");
                let result = mb.fresh_art(key, opts.theme.as_deref()).await;
                drop(spinner);
                match result {
                    Ok(art) => {
                        let _ = save_cache(&art);
                        return Ok((art, Origin::Fresh));
                    }
                    Err(e) => eprintln!("artsl: {e:#}. Showing the picture of the day instead."),
                }
            }
        }
    }

    let cached = load_cache();
    let max_age = Duration::from_secs(cfg.feed.refresh_hours.max(1) * 3600);
    if let Some((art, age)) = &cached {
        if *age <= max_age || opts.offline {
            return Ok((art.clone(), Origin::Cache));
        }
    }
    if opts.offline {
        return Ok((builtin::builtin(), Origin::Builtin));
    }

    match mailbuttons::fetch_feed(&cfg.feed.url).await {
        Ok(art) => {
            let _ = save_cache(&art);
            Ok((art, Origin::Feed))
        }
        Err(_) => match cached {
            Some((art, _)) => Ok((art, Origin::Cache)),
            None => Ok((builtin::builtin(), Origin::Builtin)),
        },
    }
}

/// A tiny stderr spinner for the slow path. Silent when stderr is not a tty.
pub struct Spinner {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Spinner {
    pub fn start(message: &'static str) -> Spinner {
        if !std::io::stderr().is_terminal() {
            return Spinner { task: None };
        }
        let task = tokio::spawn(async move {
            let glyphs = ['|', '/', '-', '\\'];
            let mut i = 0usize;
            loop {
                eprint!("\r{} {}… ", glyphs[i % glyphs.len()], message);
                let _ = std::io::stderr().flush();
                i += 1;
                tokio::time::sleep(Duration::from_millis(120)).await;
            }
        });
        Spinner { task: Some(task) }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
        }
    }
}
