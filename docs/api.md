# The art service API

Everything is under `/art/` on `mailbuttons.com`, served by `artsl serve` and
proxied by nginx (see `deploy/nginx-art.conf`). All bodies are JSON.

## Public

### `GET /art/latest.json`

The picture of the day. An art document (shape below). `Cache-Control:
public, max-age=300`, `Access-Control-Allow-Origin: *`, and an `ETag`; send
it back as `If-None-Match` and a `304` costs a few hundred bytes. If nothing
has been drawn yet the built-in picture is returned with `Cache-Control:
no-store`, so clients always get something to show.

### `GET /art/archive/{id}.json`

An earlier picture by id. Ids are `[A-Za-z0-9_-]`, under 80 characters;
anything else is 404. `Cache-Control: public, max-age=86400, immutable`.

### `GET /art/healthz`

```json
{"status":"ok","latest_id":"...","latest_age_secs":1234,"refresh_secs":21600,"drawing_slots_free":2,"version":"0.1.0"}
```

`200` while the picture of the day is younger than three refresh periods;
`503` with `status` `stale` or `no_picture_yet` otherwise. Point monitoring
at this.

## Subscribers

### `POST /art/v1/subscribe`

Trade a mailbuttons account key, once, for an art-only token.

```
Authorization: Bearer mbk_...        # from POST /api/auth/otp/cli-exchange
Content-Type: application/json

{"label": "my-laptop"}               # optional, 60 chars kept
```

`201`:

```json
{"token": "art_...", "created_at": "2026-10-29T14:00:00Z", "label": "my-laptop", "quota_per_day": 24}
```

The token is shown once and stored hashed. The account key is checked with
one authenticated call to mailbuttons and is not stored, logged or cached.
Errors: `401 missing_key`, `401 wrong_credential` (you sent an `art_` token),
`401 invalid_key`, `502 upstream`, `503 busy`.

### `DELETE /art/v1/subscribe`

`Authorization: Bearer art_...`. `204` on revoke, `401 invalid_token` if it
was not active.

### `POST /art/v1/generate`

Draw a picture now from the current headlines.

Request:

```
Authorization: Bearer art_...        # from POST /art/v1/subscribe
Content-Type: application/json

{"theme": "something hopeful"}       # optional; body may be empty; max 200 chars used
```

Response `200`: an art document, `Cache-Control: no-store`. The picture is
also written to the archive, so its `id` resolves at `/art/archive/{id}.json`.

Errors, all `{"error": "<code>", "message": "<text for a person>"}`:

| Status | `error`           | Meaning                                                        |
|--------|-------------------|----------------------------------------------------------------|
| 401    | `missing_token`   | No `Authorization: Bearer art_...` header                      |
| 401    | `subscribe_first` | You sent an account key; call `/art/v1/subscribe` first        |
| 401    | `invalid_token`   | Unknown or revoked token                                       |
| 413    |                   | Body over 4 KB                                                 |
| 429    | `quota`           | The per-token daily allowance (default 24, UTC day) is used up |
| 502    | `draw_failed`     | Feeds or the model failed; message says which                  |
| 503    | `busy`            | Waited two minutes for a drawing slot and did not get one      |

The quota is charged before drawing, so a failed draw still counts; this
keeps a broken client from costing unbounded model calls.

Timing: a drawing takes tens of seconds. Clients should allow five minutes and
show a spinner. The reference client (`artsl --fresh`) falls back to the
picture of the day on any error and tells the user why on stderr.

### How an account key is checked

At subscribe time the service calls `GET {ARTSL_MAILBUTTONS_URL}/api/v1/mailboxes`
with the account key. `200` means active; `401`/`403` means not. At most four
such checks run at once. Swap this for an entitlement check when the art
agent becomes a paid listing; the client contract does not change.

## Operators

With `ARTSL_ADMIN_TOKEN` set (otherwise these are 404):

- `GET /art/v1/admin/stats` with `Authorization: Bearer <admin token>`:
  token counts, pictures today and in total, the fifty most recent tokens
  (hash prefix, label, created, last used, revoked), and the age of the
  picture of the day.
- `POST /art/v1/admin/revoke` with `{"hash_prefix": "<8+ hex chars>"}`
  revokes every active token whose hash starts with the prefix.

Keep these off the internet (the nginx snippet denies `/art/v1/admin/`).

## Getting a key (existing mailbuttons endpoints)

```
POST /api/auth/otp/request       {"email": "you@example.com"}        -> 200
POST /api/auth/otp/cli-exchange  {"email": "...", "code": "123456"}  -> {"api_key": "mbk_..."}
```

`artsl subscribe` wraps these two calls, then calls `/art/v1/subscribe` with
the result and stores only the art token in `~/.config/artsl/config.toml`
(mode 0600).

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
| `ARTSL_ARCHIVE_DAYS` / `--archive-days`   | `90`               | Archive retention; 0 keeps everything     |
| `ARTSL_ADMIN_TOKEN` / `--admin-token`     | unset              | Enables the admin routes                  |
| `RUST_LOG`                                | `info`             | Log filter                                |
| `ARTSL_CONFIG`                            | `~/.config/artsl/config.toml` | `[generate]` model, effort, feeds |

Model and effort come from the config file (`claude-opus-5-5`, `medium`).
The request uses structured output for the document shape, leaves adaptive
thinking at its default, and opts into server-side refusal fallbacks
(`fallbacks: "default"`) on models that support them. A refusal that survives
the fallback is reported as `draw_failed`, never rendered.
