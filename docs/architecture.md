# Architecture and the review behind it

Written 2026-10-29 before the first public release. What the system is, how
signup works, what was checked for security and scale, and what is left.

## Shape

```
  terminal                                   mailbuttons.com
  ┌──────────┐   GET /art/latest.json (304)  ┌───────────────────────────────┐
  │  artsl   │ ───────────────────────────▶  │ nginx  /art/*  ──▶  artsl serve│
  │ (client) │   POST /art/v1/generate       │   cache + rate limits         │
  └──────────┘ ◀───────────────────────────  │        │          │            │
       │                                     │        │          ▼            │
       │ otp/request, otp/cli-exchange        │        │   /var/lib/artsl       │
       └────────────────────────────────────▶│ mailbuttons API  latest.json  │
                                             │ (sign-in, key    archive/     │
                                             │  check)          artsl.redb   │
                                             └───────────────────┬───────────┘
                                                                 ▼
                                                      RSS feeds   Claude API
```

Three programs share one binary and one art document format:

- **Client**: picks a picture (`source`), renders it (`scroll`). No model
  access of its own unless the user supplies a Claude key.
- **Generator**: headlines in (`news`), one Messages API call with structured
  output (`generate`), document out (`art`).
- **Service**: publishes the picture of the day, mints art tokens, draws on
  demand (`serve`, `store`).

## Signup, step by step

1. `artsl subscribe you@x` calls mailbuttons `POST /api/auth/otp/request`.
   mailbuttons emails a six-digit code. Rate limits live on that endpoint.
2. The user types the code. The client calls `POST /api/auth/otp/cli-exchange`
   and receives an account key `mbk_...`. This is the existing no-browser
   sign-in; sign-in is sign-up, and the free tier is seeded.
3. The client immediately calls `POST /art/v1/subscribe` on the art service
   with that account key as a Bearer token and a label (hostname).
4. The service checks the key with one authenticated call to mailbuttons
   (`GET /api/v1/mailboxes`), mints `art_...` (40 random alphanumerics),
   stores `sha256(token)` with created-at and label in redb, and returns the
   token once.
5. The client stores the art token in `~/.config/artsl/config.toml` (0600)
   and drops the account key. It is never written anywhere.
6. `artsl --fresh` sends the art token. `artsl unsubscribe` revokes it
   server-side and forgets it.

Why the indirection: the account key can manage the whole mailbuttons
account. A toy that scrolls pictures must not hold that on every laptop. The
art token can do one thing, is revocable from both ends, and leaks nothing
about the account.

What it costs: sign-in still mints a full account key named "CLI" on the
mailbuttons side, which artsl cannot revoke (revocation is session-only).
Two small platform changes would close this properly:

- `otp/cli-exchange` accepting `{"scope": "art"}` and minting a key that
  only the art service's check accepts.
- `GET /api/v1/whoami` for API keys, returning `customer_id`, so quota can
  be per account rather than per token, and so one account's tokens can be
  revoked together.

Both are additive, a dozen lines each in `backend/src/otp.rs` and
`backend/src/api_keys.rs`. The art service is written so that swapping the
key check for `whoami` touches one function.

## Security review

Threats considered and what answers them.

| Threat | Answer |
|---|---|
| Stolen client config | Art token only; daily quota; `unsubscribe` revokes |
| Hostile picture (feed, file, service, model) | `Art::normalise` strips control chars and escapes, caps size, replaces wide chars, validates links; applied on every load |
| Hostile server response size | Capped reads: 2 MB picture, 5 MB feed; short timeouts on the launch path |
| Terminal escape injection via caption or credit | Same normalisation; one_line strips controls |
| Prompt injection via headlines | Output constrained by JSON schema, then normalised; nothing is executed; links validated |
| Token theft from the service store | Stored hashed; plaintext exists once in a `no-store` response |
| Account key exposure at the service | Used for one upstream call, never stored or logged; 4 concurrent checks max |
| Brute-forcing tokens | 40 alphanumerics (~238 bits); lookups are local and cheap; nginx `limit_req` on write routes |
| Quota abuse | Per-token per-UTC-day counter, charged before drawing, in one write transaction |
| Resource exhaustion on the service | Body limit 4 KB; 30 s timeout on all but the drawing route; drawing bounded by a semaphore with a 120 s wait cap; MemoryMax in the unit |
| Path traversal in archive | Id restricted to `[A-Za-z0-9_-]`, under 80 chars |
| Admin surface | Off unless a token is set; digest comparison; nginx denies `/art/v1/admin/` from outside; bind is loopback |
| Supply chain | Lockfile committed; `cargo deny` in CI; rustls, no OpenSSL; MSRV job |
| Process compromise on the box | Dedicated user, `ProtectSystem=strict`, no capabilities, syscall filter, single writable directory |

Not done, on purpose: client-side certificate pinning (CDN in front makes
it brittle), encryption of the config at rest (the token is already least
privilege), and signing pictures (they are public data and normalised on
load).

## Scale

Two traffic classes with very different shapes.

**Public picture of the day.** Every `artsl` launch may fetch it, at most
once per `refresh_hours` per machine, with an ETag so the usual answer is a
304 of a few hundred bytes. nginx caches the 200 for five minutes with
`proxy_cache_lock`, so a million terminals cost the service twelve requests
an hour. With Cloudflare in front, zero. There is no state in this path; it
can be served by any number of instances or a bucket.

**On demand.** Each picture is one model call of 30 to 90 seconds. Bounded
by `concurrency` (2) and `daily_quota` (24 per token). At a thousand active
subscribers drawing twice a day that is ~2,000 calls a day, ~1.4 a minute,
well inside two slots; the 120 s wait cap turns a surge into a polite 503
rather than a pile-up. Cost scales linearly with pictures; raise the
concurrency before the quota if the queue grows.

**State.** One redb file: tokens (a few hundred bytes each) and usage rows
(purged after seven days). Tens of thousands of subscribers fit in megabytes.
A single instance owns it. This is the one thing that pins `/art/v1/*` to one
box; the picture-of-the-day path has no such pin.

## Maintenance

- Logs are structured (`tracing`), one line per event, with the token hash
  prefix never the token.
- `/art/healthz` turns 503 when the picture of the day is older than three
  refresh periods. That is the only alarm you need.
- Housekeeping runs in-process: redraw on schedule, purge old usage, trim
  archive older than `archive_days`. No cron, no second service.
- Admin stats and revoke are HTTP so they work while the service runs (redb
  is single-writer).
- Upgrades are a binary copy and a restart; tokens are forward and backward
  compatible JSON records.

## Open items

1. The two platform changes above (scoped `cli-exchange`, `whoami`).
2. Decide the product label (free with an account, or a paid listing). The
   service enforces free-with-quota today; a paid tier swaps the key check
   for an entitlement check, one function.
3. An email address for the artist (`art@mailbuttons.com` replying with the
   picture) if it should be an agent you can email like the rest of the
   crew.
