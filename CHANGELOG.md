# Changelog

All notable changes to artsl. The format follows Keep a Changelog; versions
follow semver once 1.0 is reached.

## [Unreleased]

### Fixed
- `deploy/deploy.sh` creates `/var/cache/nginx/art` for the proxy cache; the
  Debian nginx package has no `/var/cache/nginx`, so `nginx -t` failed on a
  fresh box. Its closing instructions now say `make reload-nginx`, not
  `make setup-nginx`, which overwrote the live mailbuttons.com server block.

## [0.1.1] - 2026-11-04

### Added
- Signed apt repository on GitHub Pages with amd64 and arm64 packages
  (static binaries, one package for every Debian and Ubuntu release).
- `deploy/deploy.sh`: one-command install of the art service from a release.

## [0.1.0] - 2026-11-04

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
