# Deploy on Hetzner

Run all of glot (landing + docs, dashboard, API, remote MCP, CDN, Insights) on one small VPS, with HTTPS. Everything lives in [`deploy/`](https://github.com/MattJeff/glot/tree/main/deploy). Read [Self-hosting](self-hosting.md) first for what the server does.

| Host | Serves |
|---|---|
| `example.com` | Landing and docs (Next.js). Dashboard paths (`/app`, `/login`, `/signup`, `/oauth`) redirect to `app.` |
| `app.example.com` | Dashboard (same Next.js app). Next proxies `/api` and `/cdn` to the server, so the `glot_session` cookie is same-origin (`HttpOnly; Secure; SameSite=Lax`, host-only). |
| `api.example.com` | `glot-server`: `/mcp`, `/oauth/*`, `/.well-known/*`, `/cdn/*`, `/api/insights/*`, `/api/feedback/*`, `/healthz`, and the API for the CLI (Bearer tokens). |

Four containers: `postgres:17` (not exposed), `glot-server`, `web` and `caddy:2` (Let's Encrypt, HTTP/3, zstd/gzip, HSTS and security headers). The server gets `GLOT_WEB_ORIGIN=https://app.example.com` and `GLOT_PUBLIC_URL=https://api.example.com` from `GLOT_DOMAIN`.

## 1. Order the server

In the [Hetzner Cloud console](https://console.hetzner.cloud): **CX22** (2 vCPU x86, 4 GB, 40 GB) or **CAX11** (2 vCPU ARM, 4 GB), image **Ubuntu 24.04**, add your SSH public key, keep IPv4 + IPv6. Nothing else (the firewall is set up by the bootstrap script).

## 2. DNS

At your registrar, with the server's IPs:

| Type | Name | Value |
|---|---|---|
| A | `@` | server IPv4 |
| A | `app` | server IPv4 |
| A | `api` | server IPv4 |
| AAAA | `@` | server IPv6 (the `::1` address of the `/64` shown in the console) |
| AAAA | `app` | server IPv6 |
| AAAA | `api` | server IPv6 |
| CAA | `@` | `0 issue "letsencrypt.org"` |

Check with `dig +short api.example.com` before the first deploy: Caddy requests certificates as soon as it starts.

## 3. Bootstrap (once, as root)

```sh
scp deploy/bootstrap.sh root@SERVER_IP:
ssh root@SERVER_IP bash bootstrap.sh
```

It sets UTC, upgrades packages, installs Docker Engine + the compose plugin (official repository), creates the `glot` user with your SSH key (copied from root, or `GLOT_SSH_PUBKEY="ssh-ed25519 …"`), turns off root and password login, enables `ufw` (22, 80, 443 tcp + 443 udp), `fail2ban`, unattended security upgrades and 2 GB of swap. It is idempotent. From now on: `ssh glot@SERVER_IP`.

## 4. Secrets

```sh
ssh glot@SERVER_IP
mkdir -p glot/deploy && cd glot/deploy
# paste deploy/.env.example into .env, then:
sed -i "s/^GLOT_SECRET_KEY=.*/GLOT_SECRET_KEY=$(openssl rand -hex 32)/; s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 24)/" .env
nano .env        # GLOT_DOMAIN, ACME_EMAIL
chmod 600 .env
```

Keep a copy of `GLOT_SECRET_KEY` in your password manager: it encrypts stored LLM keys, and a restored database is useless without it.

## 5. Deploy

From your Mac, at the repo root:

```sh
GLOT_HOST=SERVER_IP deploy/deploy.sh
```

It rsyncs the repo (no `.git`, `target`, `node_modules`), tags the running images `:previous`, builds the images **on the server** (native arch, no registry; the first Rust build takes ~10 min on 2 vCPU, later ones reuse the cache), runs `docker compose up -d --wait` (migrations run at server startup), installs the daily backup cron, then checks over HTTPS with certificate validation: `api/healthz`, landing, docs, `app/login`, `/.well-known/oauth-protected-resource` and `/mcp` → 401. If the check fails it rolls back to the `:previous` images. Manual rollback: `deploy/deploy.sh rollback`.

Updating = pulling the new code on your Mac and running `deploy.sh` again. Migrations only go forward: before a release with migrations, take a backup (`ssh glot@SERVER_IP glot/deploy/backup.sh`); rolling back images does not undo them.

## 6. Backups

`deploy/backup.sh` runs every day at 03:17 UTC: `pg_dump -Fc` (compressed) into `~/backups`, 14 days kept (`BACKUP_KEEP_DAYS`). Off-site copy: create a remote with `rclone config` (a Hetzner Storage Box over SFTP, or any S3-compatible bucket) in `~/.config/rclone/rclone.conf` on the server, then set `RCLONE_REMOTE=storagebox:glot` in `.env`. The remote keeps 30 days.

Restore (stops the server and web, **replaces** the database):

```sh
ssh glot@SERVER_IP glot/deploy/restore.sh ~/backups/glot-20261018T031700Z.dump
```

## 7. Monitoring

```sh
GLOT_HOST=SERVER_IP deploy/status.sh       # containers, memory, disk, last backup, error count, public checks
ssh glot@SERVER_IP 'cd glot/deploy && docker compose -f compose.prod.yml logs -f --tail 100 server'
```

Container logs rotate (5 × 10 MB per service); Caddy access logs are JSON. Add an external uptime check (UptimeRobot or Better Stack, free tier) on `https://api.example.com/healthz` (body `ok`) and `https://example.com/`.

## 8. Remote MCP and the CLI

- MCP clients (Claude, ChatGPT, Lovable…): `https://api.example.com/mcp`. See [Remote MCP](mcp-remote.md). For MCP Inspector in a browser, add its origin to `GLOT_MCP_ALLOWED_ORIGINS`.
- CLI: `[server] url = "https://api.example.com"` in `glot.toml`.
- Insights / feedback widget / OTA: `https://api.example.com` as endpoint.

## Cost

| Item | Per month |
|---|---|
| CX22 or CAX11, plus the primary IPv4 (about 0.50 €) | about 4–6 € |
| Hetzner snapshot backups (optional, +20 %) | about 1 € |
| Storage Box BX11, 1 TB (optional, off-site dumps) | about 3–4 € |
| Domain | 1 € (about 10–15 €/year) |

Check current prices on hetzner.com. Memory limits in `compose.prod.yml` (Postgres 1 GB, server 512 MB, web 512 MB, Caddy 256 MB) fit the 4 GB box; on a CX32 you can raise them.

## Try it locally

```sh
cd deploy && cp .env.example .env
# in .env: GLOT_DOMAIN=localhost, real random secrets, and the three local port lines uncommented
docker compose -f compose.prod.yml up -d --build --wait
docker compose -f compose.prod.yml cp caddy:/data/caddy/pki/authorities/local/root.crt /tmp/glot-root.crt
GLOT_DOMAIN=localhost GLOT_PORT_SUFFIX=:8443 GLOT_CURL_OPTS="--cacert /tmp/glot-root.crt" ./status.sh
docker compose -f compose.prod.yml down -v
```

Caddy uses its internal CA for `localhost`, `app.localhost` and `api.localhost`.
