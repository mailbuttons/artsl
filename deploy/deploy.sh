#!/usr/bin/env bash
# Deploy the artsl art service to the mailbuttons VPS from a GitHub release.
#
#   deploy/deploy.sh v0.1.0 [user@host]
#
# Downloads the static x86_64 Linux build of that tag, checks its SHA-256,
# installs the binary, the systemd unit and the nginx files, seeds
# /opt/artsl/artsl.env on first install (Claude key from the staff service's
# env, a fresh admin token), and starts the service. It does NOT edit the
# mailbuttons.com nginx server block or reload nginx; that is one include
# line and a reload, printed at the end so a person does it with eyes open.
set -euo pipefail

VERSION=${1:?usage: deploy/deploy.sh vX.Y.Z [user@host]}
HOST=${2:-mailbuttons@87.106.100.63}
REPO=${ARTSL_REPO:-mailbuttons/artsl}
TARGET=x86_64-unknown-linux-musl
ASSET="artsl-$VERSION-$TARGET.tar.gz"
HERE=$(cd "$(dirname "$0")" && pwd)

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "==> fetching $ASSET from $REPO"
gh release download "$VERSION" --repo "$REPO" --pattern "$ASSET" --pattern "$ASSET.sha256" --dir "$TMP"
(cd "$TMP" && shasum -a 256 -c "$ASSET.sha256")
tar -xzf "$TMP/$ASSET" -C "$TMP"
BIN="$TMP/artsl-$VERSION-$TARGET/artsl"
"$BIN" --version

echo "==> copying to $HOST"
scp -q "$BIN" "$HERE/artsl.service" "$HERE/artsl.env.example" "$HERE/nginx-art.conf" "$HERE/nginx-art-zones.conf" "$HOST:/tmp/"

echo "==> installing on $HOST"
ssh "$HOST" bash -s <<'REMOTE'
set -euo pipefail
id artsl >/dev/null 2>&1 || sudo useradd --system --home /var/lib/artsl --shell /usr/sbin/nologin artsl
sudo mkdir -p /opt/artsl/bin /var/lib/artsl
sudo chown artsl:artsl /var/lib/artsl && sudo chmod 0750 /var/lib/artsl
sudo install -m 0755 /tmp/artsl /opt/artsl/bin/artsl.new
sudo mv /opt/artsl/bin/artsl.new /opt/artsl/bin/artsl

if [ ! -f /opt/artsl/artsl.env ]; then
  KEY=""
  for f in /opt/mailbuttons/config/staff.env /opt/mailbuttons/config/archivist.env; do
    [ -r "$f" ] || sudo test -r "$f" || continue
    KEY=$(sudo grep -h '^ANTHROPIC_API_KEY=' "$f" | head -1 | cut -d= -f2- | tr -d '"' || true)
    [ -n "$KEY" ] && break
  done
  ADMIN=$(openssl rand -hex 32)
  sudo sed -e "s|^ANTHROPIC_API_KEY=.*|ANTHROPIC_API_KEY=$KEY|" \
           -e "s|^# ARTSL_ADMIN_TOKEN=.*|ARTSL_ADMIN_TOKEN=$ADMIN|" \
           /tmp/artsl.env.example | sudo tee /opt/artsl/artsl.env >/dev/null
  sudo chown root:artsl /opt/artsl/artsl.env && sudo chmod 0640 /opt/artsl/artsl.env
  if [ -z "$KEY" ]; then
    echo "!! no ANTHROPIC_API_KEY found in /opt/mailbuttons/config; edit /opt/artsl/artsl.env before the service can draw"
  else
    echo "   seeded /opt/artsl/artsl.env (Claude key from the staff service env, new admin token)"
  fi
fi

sudo install -m 0644 /tmp/artsl.service /etc/systemd/system/artsl.service
sudo install -m 0644 -D /tmp/nginx-art.conf /etc/nginx/snippets/artsl.conf
sudo install -m 0644 -D /tmp/nginx-art-zones.conf /etc/nginx/conf.d/artsl-zones.conf
# proxy_cache_path in the zones file points under /var/cache/nginx, which the
# Debian nginx package does not create; nginx -t fails with mkdir() ENOENT
# unless the parent exists. Owned by the nginx worker user.
sudo mkdir -p /var/cache/nginx/art && sudo chown www-data:www-data /var/cache/nginx/art
rm -f /tmp/artsl /tmp/artsl.service /tmp/artsl.env.example /tmp/nginx-art.conf /tmp/nginx-art-zones.conf

sudo systemctl daemon-reload
sudo systemctl enable --now artsl >/dev/null 2>&1 || true
sudo systemctl restart artsl
sleep 2
echo "   service: $(systemctl is-active artsl)"
echo "   healthz: $(curl -s http://127.0.0.1:8090/art/healthz)"
REMOTE

cat <<MSG

Service installed. Two things remain, done by hand:

1. In the mailbuttons.com server block (deployment/nginx/mailbuttons.com in
   the mailbuttons repo), add this line just before \`location /api/ {\`:

       include snippets/artsl.conf;

2. Ship it with \`make reload-nginx\` (upload, nginx -t, reload). Never
   \`make setup-nginx\`: that is the first-install bootstrap and overwrites
   the live server block with the HTTP-only file.

Then: curl -s https://mailbuttons.com/art/healthz
(503 "no_picture_yet" for the first minute or two is normal; the first draw
is in progress.)
MSG
