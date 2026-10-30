//! The art document: what the model draws, what the feed publishes, and what
//! the scroller renders. One JSON shape end to end.

use anyhow::{bail, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_width::UnicodeWidthChar;

pub const MAX_WIDTH: usize = 120;
pub const MAX_HEIGHT: usize = 30;
pub const MAX_FRAMES: usize = 8;

/// Terminal colour the picture is drawn in. Picked by the model to suit the
/// story; `orange` is the mailbuttons accent and the default for the built-in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    #[default]
    Orange,
}

impl Mood {
    pub const NAMES: [&'static str; 8] = [
        "red", "green", "yellow", "blue", "magenta", "cyan", "white", "orange",
    ];
}

/// What the model returns. Everything else on an `Art` is stamped on by us.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub title: String,
    pub caption: String,
    pub headline: String,
    pub source: String,
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub mood: Mood,
    /// One or more frames of identical size. Frame 0 is the picture; later
    /// frames differ in small details so it animates while it scrolls.
    pub frames: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Art {
    pub id: String,
    pub generated_at: String,
    #[serde(default)]
    pub model: String,
    pub title: String,
    pub caption: String,
    pub headline: String,
    pub source: String,
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub mood: Mood,
    pub frames: Vec<Vec<String>>,
}

impl Art {
    pub fn from_draft(mut d: Draft, model: &str) -> Result<Art> {
        normalise_frames(&mut d.frames)?;
        let now = Utc::now();
        let id = format!(
            "{}-{}",
            now.format("%Y%m%dT%H%M%SZ"),
            short_hash(&d.frames[0].join("\n"))
        );
        Ok(Art {
            id,
            generated_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            model: model.to_string(),
            title: one_line(&d.title, 80),
            caption: one_line(&d.caption, 110),
            headline: one_line(&d.headline, 200),
            source: one_line(&d.source, 60),
            link: d.link.trim().to_string(),
            mood: d.mood,
            frames: d.frames,
        })
    }

    /// Re-run the shape rules on art that came from disk or the network, so a
    /// hand-edited or hostile document cannot break the renderer.
    pub fn normalise(&mut self) -> Result<()> {
        normalise_frames(&mut self.frames)?;
        self.title = one_line(&self.title, 80);
        self.caption = one_line(&self.caption, 110);
        self.headline = one_line(&self.headline, 200);
        self.source = one_line(&self.source, 60);
        Ok(())
    }

    pub fn width(&self) -> usize {
        self.frames
            .first()
            .and_then(|f| f.first())
            .map(|l| l.chars().count())
            .unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.frames.first().map(|f| f.len()).unwrap_or(0)
    }

    /// One line of attribution, used after the run and by `print`.
    pub fn credit(&self) -> String {
        if self.headline.is_empty() {
            self.title.clone()
        } else if self.source.is_empty() {
            format!("{} — after: {}", self.title, self.headline)
        } else {
            format!("{} — after: {} ({})", self.title, self.headline, self.source)
        }
    }

    pub fn from_json(text: &str) -> Result<Art> {
        let mut art: Art = serde_json::from_str(text)?;
        art.normalise()?;
        Ok(art)
    }
}

/// Make every frame the same height and every line the same width, with only
/// single-width printable characters, so the scroller can treat the picture as
/// a rectangle of cells.
pub fn normalise_frames(frames: &mut Vec<Vec<String>>) -> Result<()> {
    frames.retain(|f| f.iter().any(|l| !l.trim().is_empty()));
    if frames.is_empty() {
        bail!("art has no drawable frames");
    }
    frames.truncate(MAX_FRAMES);
    for f in frames.iter_mut() {
        f.truncate(MAX_HEIGHT);
        for line in f.iter_mut() {
            *line = clean_line(line);
        }
    }
    let height = frames.iter().map(|f| f.len()).max().unwrap_or(0);
    let width = frames
        .iter()
        .flat_map(|f| f.iter())
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .min(MAX_WIDTH);
    if width == 0 || height == 0 {
        bail!("art is blank");
    }
    for f in frames.iter_mut() {
        while f.len() < height {
            f.push(String::new());
        }
        for line in f.iter_mut() {
            let mut s: String = line.chars().take(width).collect();
            let n = s.chars().count();
            s.extend(std::iter::repeat_n(' ', width - n));
            *line = s;
        }
    }
    Ok(())
}

fn clean_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        match ch {
            '\t' => out.push_str("    "),
            c if c.is_control() => {}
            c => match c.width() {
                Some(1) => out.push(c),
                Some(0) | None => {}
                Some(_) => out.push('#'),
            },
        }
    }
    out
}

pub fn one_line(s: &str, max: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = collapsed
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c.width() {
            Some(1) => c,
            _ => ' ',
        })
        .collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut t: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn short_hash(s: &str) -> String {
    let digest = Sha256::digest(s.as_bytes());
    hex::encode(&digest[..3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_become_a_rectangle() {
        let mut frames = vec![
            vec!["ab".to_string(), "abcd".to_string()],
            vec!["x".to_string()],
        ];
        normalise_frames(&mut frames).unwrap();
        assert_eq!(frames.len(), 2);
        for f in &frames {
            assert_eq!(f.len(), 2);
            for l in f {
                assert_eq!(l.chars().count(), 4);
            }
        }
    }

    #[test]
    fn wide_and_control_characters_are_tamed() {
        assert_eq!(clean_line("a\tb"), "a    b");
        assert_eq!(clean_line("a\u{1b}[31mb"), "a[31mb");
        assert_eq!(clean_line("日本"), "##");
        assert_eq!(clean_line("ok─│█"), "ok─│█");
    }

    #[test]
    fn blank_art_is_rejected() {
        let mut frames = vec![vec!["   ".to_string()]];
        assert!(normalise_frames(&mut frames).is_err());
    }

    #[test]
    fn one_line_collapses_and_truncates() {
        assert_eq!(one_line("  a \n b  ", 10), "a b");
        assert_eq!(one_line("abcdefghij", 5), "abcd…");
    }
}
