//! The two things we ask of mailbuttons.com: sign in by emailed code (which
//! yields an API key, no browser), and draw a picture now for a subscriber.
//! Plus the public feed, which needs nothing at all.

use crate::art::Art;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub struct Mailbuttons {
    base: String,
    client: reqwest::Client,
}

fn error_message(status: reqwest::StatusCode, text: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(text).ok();
    parsed
        .as_ref()
        .and_then(|v| {
            v["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("HTTP {status}"))
}

impl Mailbuttons {
    pub fn new(base: &str) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("artsl/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base: base.trim_end_matches('/').to_string(), client })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn request_otp(&self, email: &str) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/api/auth/otp/request", self.base))
            .timeout(Duration::from_secs(20))
            .json(&json!({ "email": email }))
            .send()
            .await
            .with_context(|| format!("reaching {}", self.base))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(());
        }
        match error_message(status, &text).as_str() {
            "invalid_email" => bail!("that doesn't look like an email address"),
            "too_many_requests" => bail!("too many codes requested; wait a few minutes"),
            other => bail!("mailbuttons could not send a code: {other}"),
        }
    }

    pub async fn cli_exchange(&self, email: &str, code: &str) -> Result<String> {
        let resp = self
            .client
            .post(format!("{}/api/auth/otp/cli-exchange", self.base))
            .timeout(Duration::from_secs(20))
            .json(&json!({ "email": email, "code": code }))
            .send()
            .await
            .with_context(|| format!("reaching {}", self.base))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("the code was not accepted: {}", error_message(status, &text));
        }
        let v: Value = serde_json::from_str(&text).context("unexpected reply from mailbuttons")?;
        v["api_key"]
            .as_str()
            .map(str::to_owned)
            .context("mailbuttons did not return an API key")
    }

    /// Ask the art service to draw a new picture right now. Slow by nature
    /// (it reads the news, then draws), so the timeout is generous.
    pub async fn fresh_art(&self, api_key: &str, theme: Option<&str>) -> Result<Art> {
        let mut body = json!({});
        if let Some(t) = theme {
            body["theme"] = json!(t);
        }
        let resp = self
            .client
            .post(format!("{}/art/v1/generate", self.base))
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(300))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("reaching {}", self.base))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        match status.as_u16() {
            200 => Art::from_json(&text).context("the art service sent something unreadable"),
            401 | 403 => bail!("mailbuttons did not accept your key. Run `artsl subscribe` again."),
            402 => bail!("this account is not subscribed to the art agent: {}", error_message(status, &text)),
            404 => bail!("{} has no art service at /art/v1/generate", self.base),
            429 => bail!("{}", error_message(status, &text)),
            _ => bail!("the art service failed: {}", error_message(status, &text)),
        }
    }
}

/// The public picture of the day. Short timeouts: this runs on every launch.
pub async fn fetch_feed(url: &str) -> Result<Art> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("artsl/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_millis(1500))
        .timeout(Duration::from_secs(5))
        .build()?;
    let text = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Art::from_json(&text)
}
