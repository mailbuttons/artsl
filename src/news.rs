//! Headlines from RSS 2.0 and Atom feeds, parsed with quick-xml and nothing
//! else. The model gets a short, source-balanced list; it chooses the story.

use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct Headline {
    pub title: String,
    pub summary: String,
    pub source: String,
    pub link: String,
    pub published: String,
}

/// Fetch every feed concurrently and interleave the results so one busy feed
/// cannot crowd out the others. Feeds that fail are skipped, not fatal.
pub async fn fetch_headlines(
    client: &reqwest::Client,
    feeds: &[String],
    limit: usize,
) -> Result<Vec<Headline>> {
    let mut tasks = Vec::new();
    for url in feeds {
        let client = client.clone();
        let url = url.clone();
        tasks.push(tokio::spawn(async move {
            match fetch_one(&client, &url).await {
                Ok(items) => items,
                Err(e) => {
                    eprintln!("artsl: feed skipped ({url}): {e}");
                    Vec::new()
                }
            }
        }));
    }
    let mut per_feed: Vec<Vec<Headline>> = Vec::new();
    for t in tasks {
        if let Ok(items) = t.await {
            per_feed.push(items);
        }
    }
    Ok(interleave(per_feed, limit))
}

pub const MAX_FEED_BYTES: usize = 5 * 1024 * 1024;

async fn fetch_one(client: &reqwest::Client, url: &str) -> Result<Vec<Headline>> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(12))
        .header(
            reqwest::header::USER_AGENT,
            concat!(
                "artsl/",
                env!("CARGO_PKG_VERSION"),
                " (+https://mailbuttons.com/art)"
            ),
        )
        .send()
        .await?
        .error_for_status()?;
    let bytes = read_capped(resp, MAX_FEED_BYTES).await?;
    Ok(parse_feed(&String::from_utf8_lossy(&bytes)))
}

/// Read a response body but refuse to buffer more than `max` bytes, so a
/// misbehaving or hostile server cannot make the client swallow gigabytes.
pub async fn read_capped(mut resp: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length() {
        if len as usize > max {
            anyhow::bail!("response too large ({len} bytes, limit {max})");
        }
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() + chunk.len() > max {
            anyhow::bail!("response too large (limit {max} bytes)");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

fn interleave(mut per_feed: Vec<Vec<Headline>>, limit: usize) -> Vec<Headline> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let n = per_feed.len();
    let mut i = 0;
    while n > 0 && out.len() < limit && per_feed.iter().any(|f| !f.is_empty()) {
        let feed = &mut per_feed[i % n];
        if !feed.is_empty() {
            let h = feed.remove(0);
            let key = h.title.to_lowercase();
            if !h.title.is_empty() && seen.insert(key) {
                out.push(h);
            }
        }
        i += 1;
    }
    out
}

/// A small state machine over the XML event stream. Handles `<item>` (RSS)
/// and `<entry>` (Atom), the channel/feed title as the source name, and the
/// usual synonyms for summary and date.
pub fn parse_feed(xml: &str) -> Vec<Headline> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut source = String::new();
    let mut cur: Option<Headline> = None;

    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) => {
                let name = local(e.name().as_ref());
                if name == "item" || name == "entry" {
                    cur = Some(Headline {
                        title: String::new(),
                        summary: String::new(),
                        source: String::new(),
                        link: String::new(),
                        published: String::new(),
                    });
                }
                if name == "link" {
                    if let Some(h) = cur.as_mut() {
                        if let Some(href) = attr(&e, b"href") {
                            if h.link.is_empty() {
                                h.link = href;
                            }
                        }
                    }
                }
                stack.push(name);
            }
            Event::Empty(e) => {
                let name = local(e.name().as_ref());
                if name == "link" {
                    if let Some(h) = cur.as_mut() {
                        let rel = attr(&e, b"rel").unwrap_or_else(|| "alternate".to_string());
                        if let Some(href) = attr(&e, b"href") {
                            if h.link.is_empty() && rel == "alternate" {
                                h.link = href;
                            }
                        }
                    }
                }
            }
            Event::Text(t) => {
                let text = t.unescape().map(|c| c.into_owned()).unwrap_or_default();
                absorb(&stack, &mut cur, &mut source, &text);
            }
            Event::CData(c) => {
                let text = String::from_utf8_lossy(c.into_inner().as_ref()).into_owned();
                absorb(&stack, &mut cur, &mut source, &text);
            }
            Event::End(e) => {
                let name = local(e.name().as_ref());
                if name == "item" || name == "entry" {
                    if let Some(h) = cur.take() {
                        items.push(h);
                    }
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let source = tidy_source(&source);
    for h in items.iter_mut() {
        h.source = source.clone();
        h.title = crate::art::one_line(&strip_html(&h.title), 160);
        h.summary = crate::art::one_line(&strip_html(&h.summary), 240);
        if h.summary.starts_with("Article URL:") || h.summary.starts_with("Comments URL:") {
            h.summary.clear();
        }
        h.link = h.link.trim().to_string();
        h.published = h.published.trim().to_string();
    }
    items.retain(|h| !h.title.is_empty());
    items
}

fn absorb(stack: &[String], cur: &mut Option<Headline>, source: &mut String, text: &str) {
    let leaf = match stack.last() {
        Some(l) => l.as_str(),
        None => return,
    };
    let parent = stack.iter().rev().nth(1).map(|s| s.as_str()).unwrap_or("");
    match cur.as_mut() {
        Some(h) => match leaf {
            "title" => h.title.push_str(text),
            "link" if h.link.is_empty() => h.link.push_str(text),
            "description" | "summary" | "content" | "encoded" if h.summary.is_empty() => {
                h.summary.push_str(text)
            }
            "pubDate" | "published" | "updated" | "date" if h.published.is_empty() => {
                h.published.push_str(text)
            }
            _ => {}
        },
        None => {
            if leaf == "title" && (parent == "channel" || parent == "feed") && source.is_empty() {
                source.push_str(text);
            }
        }
    }
}

/// "World news | The Guardian" -> "The Guardian"; "NPR Topics: World" -> "NPR";
/// "Ars Technica - All content" -> "Ars Technica". Feed titles are for
/// readers of feed readers, not for a caption.
pub fn tidy_source(raw: &str) -> String {
    let s = crate::art::one_line(raw, 200);
    let s = match s.rsplit_once(" | ") {
        Some((_, last)) => last.to_string(),
        None => s,
    };
    let mut cut = s.len();
    for sep in [" – ", " — ", " - ", ": "] {
        if let Some(i) = s.find(sep) {
            cut = cut.min(i);
        }
    }
    let s = s[..cut].trim();
    let s = s.strip_suffix(" Topics").unwrap_or(s);
    crate::art::one_line(s, 40)
}

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    s.rsplit(':').next().unwrap_or(&s).to_string()
}

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Good enough for feed blurbs: drop tags, decode the handful of entities
/// that survive CDATA.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
}

pub fn format_for_prompt(headlines: &[Headline]) -> String {
    let mut s = String::new();
    for (i, h) in headlines.iter().enumerate() {
        s.push_str(&format!("{}. [{}] {}\n", i + 1, h.source, h.title));
        if !h.summary.is_empty() {
            s.push_str(&format!("   {}\n", h.summary));
        }
        if !h.link.is_empty() {
            s.push_str(&format!("   {}\n", h.link));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Example News</title>
        <item><title>Cats &amp; dogs</title><link>https://x/1</link>
        <description><![CDATA[<p>A <b>story</b> about pets.</p>]]></description>
        <pubDate>Thu, 29 Oct 2026 10:00:00 GMT</pubDate></item>
        <item><title>Second</title><link>https://x/2</link></item>
        </channel></rss>"#;
        let items = parse_feed(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].source, "Example News");
        assert_eq!(items[0].title, "Cats & dogs");
        assert_eq!(items[0].summary, "A story about pets.");
        assert_eq!(
            strip_html("a</p><p>b")
                .split_whitespace()
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(items[0].link, "https://x/1");
        assert!(items[0].published.starts_with("Thu"));
    }

    #[test]
    fn parses_atom() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom Feed</title>
        <entry><title>Hello</title><link rel="alternate" href="https://a/1"/>
        <summary>Sum</summary><updated>2026-10-29T10:00:00Z</updated></entry></feed>"#;
        let items = parse_feed(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "Atom Feed");
        assert_eq!(items[0].link, "https://a/1");
        assert_eq!(items[0].summary, "Sum");
    }

    #[test]
    fn source_names_are_tidied() {
        assert_eq!(tidy_source("World news | The Guardian"), "The Guardian");
        assert_eq!(tidy_source("NPR Topics: World"), "NPR");
        assert_eq!(tidy_source("Ars Technica - All content"), "Ars Technica");
        assert_eq!(tidy_source("Hacker News: Front Page"), "Hacker News");
        assert_eq!(
            tidy_source("Al Jazeera – Breaking News, World News"),
            "Al Jazeera"
        );
        assert_eq!(tidy_source("BBC News"), "BBC News");
    }

    #[test]
    fn interleaves_and_dedupes() {
        let mk = |t: &str, s: &str| Headline {
            title: t.into(),
            summary: String::new(),
            source: s.into(),
            link: String::new(),
            published: String::new(),
        };
        let out = interleave(
            vec![
                vec![mk("a1", "A"), mk("a2", "A"), mk("a3", "A")],
                vec![mk("b1", "B"), mk("A1", "B")],
            ],
            10,
        );
        let titles: Vec<_> = out.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles, vec!["a1", "b1", "a2", "a3"]);
    }
}
