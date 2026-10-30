# Deploying the art service

One binary, one state directory, one nginx location. Ten minutes.

```bash
# On the mailbuttons box
sudo useradd --system --home /var/lib/artsl --shell /usr/sbin/nologin artsl
sudo mkdir -p /opt/artsl/bin && sudo cp artsl /opt/artsl/bin/
sudo cp deploy/artsl.env.example /opt/artsl/artsl.env   # then edit it
sudo chown root:artsl /opt/artsl/artsl.env && sudo chmod 0640 /opt/artsl/artsl.env
sudo cp deploy/artsl.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now artsl
curl -s http://127.0.0.1:8090/art/healthz
```

Add the `limit_req_zone` and `proxy_cache_path` lines from
`deploy/nginx-art.conf` to the http block, paste the location blocks into the
mailbuttons.com server block ahead of `location /api/`, then
`nginx -t && systemctl reload nginx`.

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
