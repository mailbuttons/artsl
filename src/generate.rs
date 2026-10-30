//! One Messages API call: headlines in, a console-art document out. Raw HTTP
//! because there is no official Rust SDK; structured output keeps the shape.

use crate::art::{Art, Draft, Mood};
use crate::news::{self, Headline};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2026-10-21";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub struct Generator {
    client: reqwest::Client,
    api_key: String,
    pub model: String,
    pub effort: String,
}

const SYSTEM: &str = "\
You draw console art for `artsl`, a terminal toy in the spirit of `sl`: the picture rolls \
across the user's terminal once, with a one-line caption travelling under it. You are given \
today's headlines. Choose one that lends itself to a striking, good-natured picture: an object, \
a machine, an animal, a landscape, a symbol, a scene. Not a likeness of a real person, and not \
graphic violence or suffering; for grim news prefer a quiet, symbolic image.

Draw between 44 and 90 columns wide and 9 to 24 rows tall. Use printable ASCII, and where it \
helps, single-width box-drawing and block characters (─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ ═ ║ ╔ ╗ ╚ ╝ ░ ▒ ▓ █ ▀ ▄). \
No emoji, no tabs, no characters wider than one cell. Supply 2 to 4 frames of identical size \
that differ only in small details (smoke, waves, wheels, a blinking light, a cursor, falling \
rain) so the picture animates as it moves; frame 1 must stand alone as the finished picture. \
Lines in a frame should be the same length. Give the picture a short title. The caption is one \
line under 90 characters: wry or warm, never cruel. `mood` is the colour the terminal draws it \
in. `headline`, `source` and `link` echo the story you chose, verbatim.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "caption": {"type": "string"},
            "headline": {"type": "string"},
            "source": {"type": "string"},
            "link": {"type": "string"},
            "mood": {"type": "string", "enum": Mood::NAMES},
            "frames": {
                "type": "array",
                "items": {"type": "array", "items": {"type": "string"}}
            }
        },
        "required": ["title", "caption", "headline", "source", "link", "mood", "frames"],
        "additionalProperties": false
    })
}

fn supports_fallbacks(model: &str) -> bool {
    model.starts_with("claude-opus-5")
        || model.starts_with("claude-fable")
        || model == "claude-sonnet-5-5"
}

impl Generator {
    pub fn new(api_key: String, model: String, effort: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()?;
        Ok(Self {
            client,
            api_key,
            model,
            effort,
        })
    }

    pub async fn draw(&self, headlines: &[Headline], theme: Option<&str>) -> Result<Art> {
        if headlines.is_empty() {
            bail!("no headlines to draw from");
        }
        let body = self.request_body(headlines, theme);
        let mut req = self
            .client
            .post(ANTHROPIC_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");
        if supports_fallbacks(&self.model) {
            req = req.header("anthropic-beta", FALLBACK_BETA);
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .context("calling the Claude API")?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| text.chars().take(300).collect());
            bail!("Claude API returned {status}: {msg}");
        }
        parse_response(&text, &self.model)
    }

    /// The Messages API request. Adaptive thinking is left at its default;
    /// `effort` sets the depth. Structured output pins the JSON shape.
    pub fn request_body(&self, headlines: &[Headline], theme: Option<&str>) -> Value {
        let today = chrono::Utc::now().format("%A %-d %B %Y");
        let mut user = format!(
            "Headlines for {today}:\n\n{}",
            news::format_for_prompt(headlines)
        );
        if let Some(t) = theme.map(str::trim).filter(|t| !t.is_empty()) {
            user.push_str(&format!(
                "\nThe person watching asked for this slant: {t}\n"
            ));
        }
        let mut body = json!({
            "model": self.model,
            "max_tokens": 16000,
            "system": SYSTEM,
            "messages": [{"role": "user", "content": user}],
            "output_config": {
                "effort": self.effort,
                "format": {"type": "json_schema", "schema": schema()}
            }
        });
        if supports_fallbacks(&self.model) {
            body["fallbacks"] = json!("default");
        }
        body
    }
}

/// Turn a successful Messages API response into an `Art`, or explain why not.
pub fn parse_response(text: &str, requested_model: &str) -> Result<Art> {
    let v: Value = serde_json::from_str(text).context("parsing the Claude API response")?;
    match v["stop_reason"].as_str() {
        Some("refusal") => {
            let why = v["stop_details"]["explanation"]
                .as_str()
                .unwrap_or("declined");
            bail!("the model declined to draw this one: {why}");
        }
        Some("max_tokens") => bail!("the drawing ran out of room (max_tokens); try again"),
        _ => {}
    }
    let served_by = v["model"].as_str().unwrap_or(requested_model).to_string();
    let joined: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    if joined.trim().is_empty() {
        bail!("the model returned no text");
    }
    let draft: Draft = serde_json::from_str(&joined).map_err(|e| {
        anyhow!(
            "the model's reply was not the expected JSON ({e}): {}",
            joined.chars().take(200).collect::<String>()
        )
    })?;
    Art::from_draft(draft, &served_by)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headline() -> Headline {
        Headline {
            title: "Lighthouse keeper retires after 40 years".into(),
            summary: "The last manned light on the coast goes automatic.".into(),
            source: "Example News".into(),
            link: "https://example.com/1".into(),
            published: String::new(),
        }
    }

    #[test]
    fn request_has_the_expected_shape() {
        let g = Generator::new("k".into(), "claude-opus-5-5".into(), "medium".into()).unwrap();
        let body = g.request_body(&[headline()], Some("gentle"));
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["fallbacks"], "default");
        assert_eq!(body["output_config"]["effort"], "medium");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert!(body.get("thinking").is_none());
        assert!(body.get("temperature").is_none());
        let user = body["messages"][0]["content"].as_str().unwrap();
        assert!(user.contains("Lighthouse keeper"));
        assert!(user.contains("gentle"));

        let h = Generator::new("k".into(), "claude-haiku-5-5".into(), "low".into()).unwrap();
        assert!(h
            .request_body(&[headline()], None)
            .get("fallbacks")
            .is_none());
    }

    #[test]
    fn parses_a_structured_reply() {
        let draft = json!({
            "title": "The Last Light",
            "caption": "Forty years of not letting anyone hit the rocks.",
            "headline": "Lighthouse keeper retires after 40 years",
            "source": "Example News",
            "link": "https://example.com/1",
            "mood": "yellow",
            "frames": [["  *  ", " /|\\ ", "/_|_\\"], ["  .  ", " /|\\ ", "/_|_\\"]]
        });
        let resp = json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "stop_reason": "end_turn", "stop_details": null,
            "content": [
                {"type": "thinking", "thinking": "", "signature": "x"},
                {"type": "text", "text": draft.to_string()}
            ]
        });
        let art = parse_response(&resp.to_string(), "claude-opus-5-5").unwrap();
        assert_eq!(art.title, "The Last Light");
        assert_eq!(art.model, "claude-opus-5-5");
        assert_eq!(art.mood, Mood::Yellow);
        assert_eq!(art.frames.len(), 2);
        assert_eq!(art.width(), 5);
        assert!(art.id.len() > 10);
    }

    #[test]
    fn refusal_is_reported_not_rendered() {
        let resp = json!({
            "model": "claude-opus-5-5", "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": "general_harms", "explanation": "no"},
            "content": []
        });
        let err = parse_response(&resp.to_string(), "claude-opus-5-5").unwrap_err();
        assert!(err.to_string().contains("declined"));
    }
}
