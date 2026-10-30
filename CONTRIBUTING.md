# Contributing

Thanks for looking. artsl is small on purpose; please keep it that way.

## Setting up

```bash
git clone https://github.com/mailbuttons/artsl && cd artsl
cargo test
ARTSL_CONFIG=/tmp/c.toml ARTSL_CACHE_DIR=/tmp/cache cargo run -- --builtin
```

Rust 1.82 or newer. No other toolchain is needed. A Claude API key is only
needed for `generate`, `publish` and `serve`.

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three plus `cargo deny` and a build at the minimum
supported Rust version.

## What goes where

| Module          | Owns                                                      |
|-----------------|-----------------------------------------------------------|
| `art`           | The document, normalisation, the only place sizes are set |
| `builtin`       | The shipped picture                                       |
| `scroll`        | Rendering. No network, no files                           |
| `source`        | Which picture to show, the cache                          |
| `news`          | RSS/Atom parsing, capped downloads                        |
| `generate`      | The one Messages API call                                 |
| `mailbuttons`   | Sign-in and the art service, client side                  |
| `serve`         | The art service                                           |
| `store`         | Tokens and quota (redb)                                   |
| `config`        | Config file and paths                                     |

## Pictures

If you improve the built-in picture, keep it pure ASCII and keep every line
the same length across frames (the test enforces this). If you add a sample
under `examples/`, say which model drew it and when.

## Changes to the prompt

The system prompt in `generate.rs` is short on purpose. Before changing it,
draw a handful of pictures with the old and new prompt and put a couple of
each in the pull request so reviewers can see the difference rather than
guess.

## Conduct

Be kind. Decisions on this project are made by its maintainers; disagreement
is welcome, rudeness is not.
