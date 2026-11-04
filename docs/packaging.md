# Publishing and packaging

The order below goes from the cheapest channel to the ones with gatekeepers.
Each step assumes the previous one.

## 0. Before the first tag

- Create the GitHub repository (`mailbuttons/artsl` is what `Cargo.toml`
  says; change both if it lives elsewhere) and push `main`.
- `cargo publish --dry-run` passes locally.
- Decide the maintainer address in `Cargo.toml` `[package.metadata.deb]` and
  `packaging/PKGBUILD`.
- Make sure the service is deployed, so a fresh install has a picture of the
  day to show. Without it the tool still works (built-in picture).

## 1. crates.io

```bash
cargo login            # once
cargo publish --dry-run
git tag v0.1.0 && git push --tags
cargo publish
```

Users: `cargo install artsl`. The release workflow runs on the tag and
attaches static musl binaries for x86_64 and aarch64, macOS binaries, a
`.deb`, and SHA-256 sums to the GitHub release.

## 2. Debian and Ubuntu

Done, today: every release publishes amd64 and arm64 `.deb`s (static musl
binaries, so one package serves every Debian and Ubuntu release) to a
signed apt repository on GitHub Pages, `https://mailbuttons.github.io/artsl/apt`.
The `apt-repo` job in `.github/workflows/release.yml` builds it with
`dpkg-scanpackages` and `apt-ftparchive` and signs `InRelease` with the key
from `deploy/apt/make-signing-key.sh` (secrets `APT_GPG_PRIVATE_KEY` and
`APT_GPG_FINGERPRINT`; the key expires after five years, re-run the script
to rotate). To republish an existing tag, run the workflow by hand with
that tag. A custom host such as `apt.mailbuttons.com` is a CNAME to
`mailbuttons.github.io` plus the domain in the Pages settings; the
`signed-by` line in the README changes accordingly.

Official path: Debian packages Rust crates through the Rust team with
`debcargo`. Requirements that this crate already meets: all dependencies
on crates.io, a lockfile, a permissive licence, no vendored C. File an ITP
(Intent To Package) bug against `wnpp`, then work with the Debian Rust team
on `debcargo-conf`. Expect weeks, and a version pin to whatever dependency
versions Debian carries. Ubuntu inherits from Debian.

## 3. Arch Linux

`packaging/PKGBUILD` builds from the tagged tarball. Publish to the AUR:

```bash
git clone ssh://aur@aur.archlinux.org/artsl.git aur-artsl
cp packaging/PKGBUILD aur-artsl/ && cd aur-artsl
updpkgsums && makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO && git commit -m "artsl 0.1.0" && git push
```

## 4. Homebrew

A formula in a tap (`mailbuttons/homebrew-tap`) pointing at the tarball,
`depends_on "rust" => :build`, `system "cargo", "install", *std_cargo_args`,
plus `generate_completions_from_executable(bin/"artsl", "completions")` and
`man1.install Utils.safe_popen_read(bin/"artsl", "man")`. Homebrew core
accepts notable projects after they have some history; start with the tap.

## 5. Fedora, openSUSE, NixOS

- Fedora: `rust2rpm artsl` generates a spec; submit through a package review.
- openSUSE: `cargo vendor` into an OBS project.
- NixOS: a `buildRustPackage` derivation with `cargoHash`; submit to nixpkgs.

## Release checklist

1. `CHANGELOG.md` has a section for the version.
2. `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`.
3. Bump `version` in `Cargo.toml` and `pkgver` in `packaging/PKGBUILD`.
4. Draw one picture with the release binary and look at it.
5. Tag, push, let the workflow build, check the sums, then `cargo publish`.
6. Update the AUR package.
