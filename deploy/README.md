# Deploying the art service

One binary, one state directory, one nginx include. Ten minutes.

```bash
# From this checkout, with `gh` logged in and SSH access to the box
deploy/deploy.sh v0.1.0
```

That downloads the tagged static build, verifies its checksum, installs the
binary, the systemd unit, the nginx snippet and zones, seeds
`/opt/artsl/artsl.env` on first run (Claude key from the staff service's env,
a fresh admin token) and starts the service. It stops short of touching the
live server block. Finish by hand:

1. Add `include snippets/artsl.conf;` to the mailbuttons.com server block
   ahead of `location /api/` (in the mailbuttons repo,
   `deployment/nginx/mailbuttons.com`, shipped with `make reload-nginx`; never `make setup-nginx`, which overwrites the live block with the pre-SSL bootstrap).
2. `curl -s https://mailbuttons.com/art/healthz` should return ok.

Upgrades are the same command with the new tag. By hand, the steps are:
create the `artsl` user, copy the binary to `/opt/artsl/bin`, the env to
`/opt/artsl/artsl.env` (0640, root:artsl), the unit to
`/etc/systemd/system`, `nginx-art.conf` to `/etc/nginx/snippets/artsl.conf`
and `nginx-art-zones.conf` to `/etc/nginx/conf.d/`, then enable the unit.

## Watching it

- `journalctl -u artsl -f` for structured logs: every redraw, every
  subscription (token hash prefix and label), every on-demand picture with
  its duration, every failure with its cause.
- `GET /art/healthz` returns 200 while the picture of the day is younger
  than three refresh periods, 503 otherwise. Point your uptime monitor at it.
- With `ARTSL_ADMIN_TOKEN` set, from the box:
  ```bash
  curl -s -H "Authorization: Bearer $ARTSL_ADMIN_TOKEN" http://127.0.0.1:8090/art/v1/admin/stats | jq
  curl -s -X POST -H "Authorization: Bearer $ARTSL_ADMIN_TOKEN" \
       -d '{"hash_prefix":"1a2b3c4d5e6f"}' http://127.0.0.1:8090/art/v1/admin/revoke
  ```

## Upgrading

Copy the new binary over `/opt/artsl/bin/artsl` and `systemctl restart artsl`.
State (`latest.json`, `archive/`, `artsl.redb`) is untouched. The store is a
single redb file; back it up with the rest of `/var/lib/artsl` (it is small).

## Rolling back

Keep the previous binary beside the new one. Tokens issued by a newer version
are plain records; an older version reads them.

## Scaling

The picture of the day is a 3 KB JSON file that nginx caches for five
minutes, so the public path costs the service one request per five minutes
regardless of how many terminals ask. Put Cloudflare in front and it costs
nothing. On-demand drawing is bounded by `ARTSL_CONCURRENCY` and the daily
quota; each picture is one model call of 30 to 90 seconds. A single small VM
is the right size. If demand ever outgrows it, the service runs happily
behind a second instance for the public routes; only `/art/v1/*` must stay on
one instance because the token store is a local file.

## The publish-only alternative

If you do not want on-demand drawing yet, skip `artsl.service`, install
`artsl-publish.service` and `artsl-publish.timer`, and serve `/var/lib/artsl`
as static files (commented block in `nginx-art.conf`). No tokens, no store,
no admin.
