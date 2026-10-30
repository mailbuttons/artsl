//! `~/.config/artsl/config.toml` and `~/.cache/artsl/`. Every field has a
//! default, so a missing file is a valid configuration.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_MAILBUTTONS_URL: &str = "https://mailbuttons.com";
pub const DEFAULT_FEED_URL: &str = "https://mailbuttons.com/art/latest.json";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

pub const DEFAULT_NEWS_FEEDS: &[&str] = &[
    "https://feeds.bbci.co.uk/news/world/rss.xml",
    "https://www.theguardian.com/world/rss",
    "https://feeds.npr.org/1004/rss.xml",
    "https://www.aljazeera.com/xml/rss/all.xml",
    "https://feeds.arstechnica.com/arstechnica/index",
    "https://hnrss.org/frontpage",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub mailbuttons: MailbuttonsCfg,
    pub feed: FeedCfg,
    pub display: DisplayCfg,
    pub generate: GenerateCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MailbuttonsCfg {
    /// Where `subscribe` signs in and where `--fresh` asks for a picture.
    pub base_url: String,
    /// The `mbk_` key from `artsl subscribe`. Presence means subscribed.
    pub api_key: Option<String>,
    pub email: Option<String>,
}

impl Default for MailbuttonsCfg {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_MAILBUTTONS_URL.to_string(),
            api_key: None,
            email: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedCfg {
    /// Public, pre-drawn picture of the day. No account needed.
    pub url: String,
    /// How long a cached copy is trusted before the feed is asked again.
    pub refresh_hours: u64,
}

impl Default for FeedCfg {
    fn default() -> Self {
        Self {
            url: DEFAULT_FEED_URL.to_string(),
            refresh_hours: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayCfg {
    /// Milliseconds per column of travel.
    pub speed_ms: u64,
    /// Ticks each animation frame is held for.
    pub frame_hold: u64,
    /// Like `sl`: ignore q and Ctrl-C until the picture has gone by.
    pub stubborn: bool,
}

impl Default for DisplayCfg {
    fn default() -> Self {
        Self {
            speed_ms: 28,
            frame_hold: 6,
            stubborn: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerateCfg {
    pub model: String,
    /// `low` | `medium` | `high` | `xhigh` | `max`
    pub effort: String,
    /// RSS/Atom feeds the pictures are drawn from.
    pub feeds: Vec<String>,
    /// How many headlines the model is shown.
    pub headlines: usize,
    /// Falls back to `ANTHROPIC_API_KEY` when unset.
    pub anthropic_api_key: Option<String>,
}

impl Default for GenerateCfg {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            effort: "medium".to_string(),
            feeds: DEFAULT_NEWS_FEEDS.iter().map(|s| s.to_string()).collect(),
            headlines: 14,
            anthropic_api_key: None,
        }
    }
}

pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ARTSL_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("artsl")
        .join("config.toml")
}

pub fn cache_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("ARTSL_CACHE_DIR") {
        return PathBuf::from(p);
    }
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("artsl")
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn anthropic_api_key(&self) -> Option<String> {
        self.generate
            .anthropic_api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .filter(|k| !k.trim().is_empty())
    }

    pub fn is_subscribed(&self) -> bool {
        self.mailbuttons
            .api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}
