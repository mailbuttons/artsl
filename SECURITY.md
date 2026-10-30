# Security

## Reporting

Email security@codecutter.io. Say "artsl" in the subject. You will get a
reply within three working days, and a fix or a plan within fourteen. Please
do not open a public issue for anything that could be exploited.

## What the client protects

- **Only an art-only token is stored.** `artsl subscribe` uses the mailbuttons
  account key exactly once, to obtain an `art_` token from the art service,
  and never writes the account key to disk. A stolen config file lets someone
  draw your daily allowance of pictures. It cannot read mail, create
  mailboxes or spend money. Revoke with `artsl unsubscribe`.
- **Config is written with mode 0600** and atomic replace. The cache is public
  data.
- **Every picture is normalised before it touches the terminal**, whether it
  came from the feed, a file, the service or the model: control characters
  and escape sequences are stripped, wide characters replaced, sizes capped,
  links restricted to absolute http(s). A hostile document cannot inject
  terminal escapes.
- **Downloads are capped** (2 MB for a picture, 5 MB per news feed) and
  time-limited, so a misbehaving server cannot exhaust memory or hang launch.
- **TLS is rustls.** There is no OpenSSL and no C code in the dependency tree
  beyond what the Rust standard library links.
- **Nothing runs at launch but one GET** of the public feed, and only when the
  cache is older than `refresh_hours`.

## What the service protects

- Tokens are stored as SHA-256 hashes. The plaintext exists only in the
  response that mints it.
- Account keys are checked with mailbuttons once, at subscribe time, through
  a bounded number of concurrent upstream calls. They are not stored, logged
  or cached.
- Per-token daily quota, bounded concurrency for drawing, 4 KB request body
  limit, 30 s timeout on every route except drawing, archive ids restricted
  to `[A-Za-z0-9_-]`.
- Admin routes exist only when `ARTSL_ADMIN_TOKEN` is set, and bind-address
  defaults to loopback.
- The systemd unit runs as a dedicated user with `ProtectSystem=strict`,
  `NoNewPrivileges`, a private tmp and a read-write path limited to its state
  directory. Put per-IP rate limits in nginx (`deploy/nginx-art.conf`).
- Headlines are untrusted input to the model. The output is constrained by a
  JSON schema and then normalised like any other picture; nothing the model
  returns is executed or rendered raw.

## Known limits, and the platform change that lifts them

- Signing in mints a full account key on mailbuttons ("CLI") that artsl
  discards but cannot revoke, because revocation is session-only. Until
  mailbuttons offers scoped keys on `otp/cli-exchange`, users can revoke it
  in the dashboard. The art token has no such power.
- Quota is per token, not per account, because mailbuttons has no API-key
  `whoami`. A person can obtain several tokens by signing in several times
  (one emailed code each). The cost is bounded per token per day.

## Supply chain

`Cargo.lock` is committed. CI runs `cargo deny` (advisories, licences,
sources) on every push, and release binaries are built from a tag by the
public workflow with checksums alongside.
