# The art service API

Everything is under `/art/` on `mailbuttons.com`, served by `artsl serve` and
proxied by nginx (see `deploy/nginx-art.conf`). All bodies are JSON.

## Public

### `GET /art/latest.json`

The picture of the day. An art document (shape below). `Cache-Control:
public, max-age=300`, `Access-Control-Allow-Origin: *`. If nothing has been
drawn yet the built-in picture is returned with `Cache-Control: no-store`, so
clients always get something to show.

### `GET /art/archive/{id}.json`

An earlier picture by id. Ids are `[A-Za-z0-9_-]`, under 80 characters;
anything else is 404. `Cache-Control: public, max-age=86400, immutable`.

### `GET /art/healthz`

`ok`.

## Subscribers

### `POST /art/v1/generate`

Draw a picture now from the current headlines.

Request:

```
Authorization: Bearer mbk_...        # from POST /api/auth/otp/cli-exchange
Content-Type: application/json

{"theme": "something hopeful"}       # optional; body may be empty; max 200 chars used
```

Response `200`: an art document, `Cache-Control: no-store`. The picture is
also written to the archive, so its `id` resolves at `/art/archive/{id}.json`.

Errors, all `{"error": "<code>", "message": "<text for a person>"}`:

| Status | `error`        | Meaning                                                        |
|--------|----------------|----------------------------------------------------------------|
| 401    | `missing_key`  | No `Authorization: Bearer` header                              |
| 401    | `invalid_key`  | mailbuttons rejected the key                                   |
| 429    | `quota`        | The per-key daily allowance (default 24, UTC day) is used up   |
| 502    | `upstream`     | mailbuttons could not be reached to check the key              |
| 502    | `draw_failed`  | Feeds or the model failed; message says which                  |
| 503    | `busy`         | Waited two minutes for a drawing slot and did not get one      |

Timing: a drawing takes tens of seconds. Clients should allow five minutes and
show a spinner. The reference client (`artsl --fresh`) falls back to the
picture of the day on any error and tells the user why on stderr.

### How a key is checked

The service calls `GET {ARTSL_MAILBUTTONS_URL}/api/v1/mailboxes` with the same
Bearer token. `200` means active; `401`/`403` means not. A positive answer is
remembered for ten minutes per key (keys are stored only as SHA-256 hashes in
memory). Swap this for an entitlement check when the art agent becomes a paid
listing; the client contract does not change.

## Getting a key (existing mailbuttons endpoints)

```
POST /api/auth/otp/request       {"email": "you@example.com"}        -> 200
POST /api/auth/otp/cli-exchange  {"email": "...", "code": "123456"}  -> {"api_key": "mbk_..."}
```

`artsl subscribe` wraps these two calls and stores the key in
`~/.config/artsl/config.toml` (mode 0600).

## The art document

```json
{
  "id": "20261029T080000Z-3f9a1c",
  "generated_at": "2026-10-29T08:00:00Z",
  "model": "claude-opus-5-5",
  "title": "string, one line, <= 80 chars",
  "caption": "string, one line, <= 110 chars",
  "headline": "the story it was drawn from, verbatim",
  "source": "publisher name",
  "link": "https://...",
  "mood": "red | green | yellow | blue | magenta | cyan | white | orange",
  "frames": [["line", "line", "..."], ["line", "line", "..."]]
}
```

Guarantees after normalisation (which both the server and the client apply):
1 to 8 frames; every frame the same height (1 to 30 rows); every line the same
width (1 to 120 columns); only single-width printable characters. Frame 0 is
the finished picture; later frames differ in small details for animation.

## Server configuration

| Variable / flag                           | Default            | Purpose                                   |
|-------------------------------------------|--------------------|-------------------------------------------|
| `ANTHROPIC_API_KEY`                       | required           | Drawing                                   |
| `ARTSL_DATA_DIR` / `--data-dir`           | required           | `latest.json` and `archive/`              |
| `ARTSL_LISTEN` / `--listen`               | `127.0.0.1:8090`   | Bind address                              |
| `ARTSL_MAILBUTTONS_URL` / `--mailbuttons-url` | `https://mailbuttons.com` | Where keys are checked         |
| `ARTSL_REFRESH_HOURS` / `--refresh-hours` | `6`                | Picture-of-the-day cadence                |
| `ARTSL_DAILY_QUOTA` / `--daily-quota`     | `24`               | Fresh pictures per key per UTC day        |
| `ARTSL_CONCURRENCY` / `--concurrency`     | `2`                | Simultaneous drawings                     |
| `ARTSL_CONFIG`                            | `~/.config/artsl/config.toml` | `[generate]` model, effort, feeds |

Model and effort come from the config file (`claude-opus-5-5`, `medium`).
The request uses structured output for the document shape, leaves adaptive
thinking at its default, and opts into server-side refusal fallbacks
(`fallbacks: "default"`) on models that support them. A refusal that survives
the fallback is reported as `draw_failed`, never rendered.
