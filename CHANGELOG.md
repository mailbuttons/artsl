# Changelog

All notable changes to artsl. The format follows Keep a Changelog; versions
follow semver once 1.0 is reached.

## [Unreleased]

### Added
- First release: the scroller, the built-in picture, the picture of the day
  from mailbuttons.com, `subscribe` by emailed code, `--fresh` on-demand
  drawing, `generate` with your own Claude key, `serve` and `publish` for
  the service side.
- Art-only tokens: the client keeps an `art_` token, never the account key.
- Conditional feed fetches (ETag / 304), capped downloads, link validation.
- `completions` and `man` subcommands for packagers.
- Service: redb-backed tokens and quota, health endpoint with staleness,
  archive trimming, admin stats and revoke behind an optional token,
  structured logs.
