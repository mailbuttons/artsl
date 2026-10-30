# artsl

Console art that rolls across your terminal, drawn by an LLM from today's news.
A homage to [`sl`](https://github.com/mtoyoda/sl), with a picture that changes
every few hours.

```
         .-----------------------------------------------.
         |  >_ artsl                                [x]  |
         |-----------------------------------------------|
         |    (o)-------(o)-------(o)                    |
         |      \      /   \      /      .-----------.   |
         |       (o)--'     '--(o)       | hello,    |   |
         |      /      \   /      \      | world_    |   |
         |    (o)-------(o)-------(o)    '-----------'   |
         |         \      |      /                       |
         |          `--[ LLM ]--'    . o O ( news )      |
         '-----------------------------------------------'
    _________|_______________________________________________|_____
   |_________________________________________________________________|
      (__)       (__)                                 (__)       (__)

  Every run after this one is drawn fresh from the day's news.
```

That is the picture it ships with. Every run after the first one shows the
**picture of the day**: a scene drawn by Claude from the morning's headlines,
published at `https://mailbuttons.com/art/latest.json` and redrawn every few
hours. Subscribers can have one drawn **on the spot** from whatever the news is
right now.

## Install

```bash
cargo install --path .        # from this checkout
# or, once published:
cargo install artsl
```

Rust 1.80 or newer. One static binary, no runtime dependencies.

## Use

```bash
artsl                 # roll the picture of the day across the terminal
artsl --fresh         # have a new one drawn now from the latest news (subscribers)
artsl -f -t "something hopeful"   # with a slant
artsl --builtin       # the picture it ships with
artsl --offline       # whatever is cached, never the network
artsl --stubborn      # like sl: q and Ctrl-C do nothing until it has gone by
artsl print           # the current picture, no animation (pipes, READMEs)
artsl status          # config, cache age, subscription
```

Press `q`, `Esc` or `Ctrl-C` to stop it early, unless you asked for `--stubborn`.

### Subscribe

```bash
artsl subscribe you@example.com
```

mailbuttons emails you a six-digit code; type it in and the tool stores an API
key in its config file. No browser, no password. From then on `artsl --fresh`
asks the art service to read the news and draw a new picture just for you
(allowance: a couple of dozen a day). `artsl unsubscribe` forgets the key.

The picture of the day never needs an account.

### Draw with your own key

If you have a Claude API key you can skip the service entirely:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
artsl generate --show                 # draw and roll it
artsl generate --out today.json       # keep it
artsl --file today.json               # roll a saved picture
artsl news                            # the headlines it would draw from
```

A real one, drawn by `claude-opus-5-5` on 29 October 2026 from a Hacker News
headline about Mistral's "Le Chonk" release, is in
[examples/le-chonk-lifts.json](examples/le-chonk-lifts.json):
`artsl --file examples/le-chonk-lifts.json`.

## How it picks a picture

1. `--file` or `--builtin`, if you asked.
2. `--fresh`: drawn on demand through your subscription. Falls through on any
   error, with a note on stderr.
3. The cached copy of the picture of the day, if younger than `refresh_hours`.
4. The public feed, fetched with short timeouts so launch stays snappy.
5. The cached copy however old.
6. The built-in picture.

Nothing blocks for long on the default path: the feed fetch gives up after a
few seconds and the built-in picture takes over.

## Configuration

`~/.config/artsl/config.toml` (`artsl config-path` prints it; `ARTSL_CONFIG`
overrides). Every key is optional.

```toml
[mailbuttons]
base_url = "https://mailbuttons.com"
# api_key and email are written by `artsl subscribe`

[feed]
url = "https://mailbuttons.com/art/latest.json"
refresh_hours = 3

[display]
speed_ms = 28       # per column of travel
frame_hold = 6      # ticks per animation frame
stubborn = false

[generate]           # only for `generate`, `publish` and `serve`
model = "claude-opus-5-5"
effort = "medium"   # low | medium | high | xhigh | max
headlines = 14
feeds = [
  "https://feeds.bbci.co.uk/news/world/rss.xml",
  "https://www.theguardian.com/world/rss",
  "https://feeds.npr.org/1004/rss.xml",
  "https://www.aljazeera.com/xml/rss/all.xml",
  "https://feeds.arstechnica.com/arstechnica/index",
  "https://hnrss.org/frontpage",
]
# anthropic_api_key = "sk-ant-..."   # or the ANTHROPIC_API_KEY environment variable
```

The cache lives in `~/.cache/artsl/latest.json` (`ARTSL_CACHE_DIR` overrides).

## The art document

One JSON shape end to end. The model fills in the first seven fields through
structured output; `id`, `generated_at` and `model` are stamped on afterwards.

```json
{
  "id": "20261029T080000Z-3f9a1c",
  "generated_at": "2026-10-29T08:00:00Z",
  "model": "claude-opus-5-5",
  "title": "The Last Light",
  "caption": "Forty years of not letting anyone hit the rocks.",
  "headline": "Lighthouse keeper retires after 40 years",
  "source": "BBC News",
  "link": "https://...",
  "mood": "yellow",
  "frames": [["...", "..."], ["...", "..."]]
}
```

`frames` is a list of frames, each a list of lines. On load every frame is
made the same height and every line the same width, tabs and control
characters are removed, and anything wider than one terminal cell becomes `#`,
so a hand-edited or hostile document cannot upset the renderer. `mood` is one
of `red green yellow blue magenta cyan white orange`.

## Running the service (the mailbuttons.com side)

The same binary is the server. Full contract in [docs/api.md](docs/api.md).

```bash
ANTHROPIC_API_KEY=... artsl serve --data-dir /var/lib/artsl --listen 127.0.0.1:8090
```

- `GET /art/latest.json` is the public picture of the day, redrawn every
  `--refresh-hours` (6) by a background task.
- `POST /art/v1/generate` with `Authorization: Bearer mbk_...` draws one now.
  The key is checked against mailbuttons with one authenticated call (cached
  ten minutes), then a per-key daily quota applies (`--daily-quota`, 24), then
  at most `--concurrency` (2) pictures are drawn at once.
- `GET /art/archive/{id}.json` serves any earlier picture.

Deployment files are in [deploy/](deploy/): a systemd unit for `serve`, an
alternative `publish` timer that only maintains the static feed, an env
example, and the nginx `location /art/` block for the mailbuttons.com server.

A key point for the platform: the service does not touch the mailbuttons
database. It trusts a key because the mailbuttons API accepted it. To make the
art agent a paid listing later, swap that check for an entitlement endpoint;
nothing else changes. A draft catalogue card is in
[docs/catalogue-card.md](docs/catalogue-card.md).

## What leaves your machine

- Plain `artsl`: one `GET` of the public feed, at most every few hours, with a
  `User-Agent` of `artsl/<version>`.
- `artsl subscribe`: your email address, to mailbuttons, to send the code.
- `artsl --fresh`: your mailbuttons key and the optional `--theme` text.
- `artsl generate`: the headlines and your theme, to the Claude API, with your
  own key.

## Development

```bash
cargo test
cargo clippy --all-targets
ARTSL_CONFIG=/tmp/c.toml ARTSL_CACHE_DIR=/tmp/cache cargo run -- --builtin
```

Modules: `art` (document and normalisation), `builtin` (the shipped picture),
`scroll` (the renderer), `source` (which picture to show), `news` (RSS/Atom),
`generate` (the Messages API call), `mailbuttons` (sign-in and on-demand
client), `serve` (the HTTP service), `config`.

## Credits

`sl` is by Toyoda Masashi, and remains the better joke. The pictures are drawn
by Claude; the headlines belong to the publishers linked in each one.

MIT licence.
