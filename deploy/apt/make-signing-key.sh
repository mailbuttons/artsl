#!/usr/bin/env bash
# One-time: create the apt repository signing key and hand it to the release
# workflow as repository secrets. Run from a machine with gh logged in.
#
#   deploy/apt/make-signing-key.sh
#
# The private key stays in your local GnuPG keyring and in the repo secret
# APT_GPG_PRIVATE_KEY; the public key is written to deploy/apt/ for the
# repository and served as key.gpg from the apt site. Expires in five years:
# re-run this script before then to rotate (users re-fetch key.gpg).
set -euo pipefail
REPO=${ARTSL_REPO:-mailbuttons/artsl}
UID_EMAIL=${ARTSL_APT_KEY_EMAIL:-apt@mailbuttons.com}
HERE=$(cd "$(dirname "$0")" && pwd)

if gpg --batch --list-secret-keys "$UID_EMAIL" >/dev/null 2>&1; then
  echo "==> using the existing key for $UID_EMAIL in your keyring"
else
  echo "==> generating a new ed25519 signing key for $UID_EMAIL"
  gpg --batch --quiet --gen-key <<PARAMS
%no-protection
Key-Type: eddsa
Key-Curve: ed25519
Key-Usage: sign
Name-Real: mailbuttons artsl apt repository
Name-Email: $UID_EMAIL
Expire-Date: 5y
%commit
PARAMS
fi

FPR=$(gpg --batch --with-colons --list-secret-keys "$UID_EMAIL" | awk -F: '/^fpr/{print $10; exit}')
gpg --batch --armor --export "$FPR" > "$HERE/mailbuttons-artsl.asc"
gpg --batch --armor --export-secret-keys "$FPR" | gh secret set APT_GPG_PRIVATE_KEY --repo "$REPO"
gh secret set APT_GPG_FINGERPRINT --repo "$REPO" --body "$FPR"

echo "==> fingerprint $FPR"
echo "    public key: deploy/apt/mailbuttons-artsl.asc (commit it)"
echo "    secrets set on $REPO: APT_GPG_PRIVATE_KEY, APT_GPG_FINGERPRINT"
echo "    back it up:  gpg --armor --export-secret-keys $FPR > artsl-apt-key.asc  (keep offline)"
