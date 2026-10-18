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

`deploy/backup.sh` runs every day at 03:17 UTC (log: `~/backups/backup.log`):

1. `pg_dump -Fc` into `~/backups/glot-<UTC>.dump`, checked with `pg_restore -l`. Plain, for fast local restores.
2. A bundle `glot-<UTC>.tar.gz.age`: the dump, `deploy/.env` and the Caddy volumes (certificates), encrypted with [age](https://age-encryption.org) to `AGE_RECIPIENT`, a **public** key. The server can encrypt but never decrypt: someone who takes the server or the bucket cannot read old backups.
3. If `RCLONE_REMOTE` is set: `rclone copy` (never `sync`) to `daily/`, and on the 1st of the month to `monthly/`, then `rclone check` of size and hash against the remote copy. Remote retention: 30 days in `daily/`, 12 months in `monthly/`. Local: 14 days (`BACKUP_KEEP_DAYS`).

Any failure exits non-zero, sends an email through Resend when `RESEND_API_KEY` is set (to `BACKUP_ALERT_TO`, default `ACME_EMAIL`), and pings `HEALTHCHECK_URL/fail` when set. Set `HEALTHCHECK_URL` to a [healthchecks.io](https://healthchecks.io) check (free, period 1 day) to get alerted when the backup **stops running** too.

Visual reports and screenshots live in Postgres, so the dump covers them. Hetzner snapshots are not a backup: they die with the account.

### The age key

Generate it once on your Mac, never on the server:

```sh
brew install age
(umask 077; age-keygen -o ~/.config/glot/backup-age.key)   # prints the public key
age-keygen -y ~/.config/glot/backup-age.key                  # public key again -> AGE_RECIPIENT in deploy/.env
```

**Copy the private key file into your password manager now** (it is one line, `AGE-SECRET-KEY-1…`). Without it every off-site backup is noise; if the Mac dies with the only copy, so do the backups. Keep `GLOT_SECRET_KEY` there too (it is also inside the encrypted `.env`).

### Off-site destination

rclone reads its remote from environment variables in `deploy/.env` (`RCLONE_CONFIG_<NAME>_*`), no `rclone.conf`. `sudo apt install age rclone` on the server, then append one of:

- **Hetzner Storage Box** (BX11, about 3 €/month, recommended): order it in the Hetzner console (same location as the VPS), enable *SSH support* and *External reachability* in its settings, then follow `deploy/offsite/storagebox.env.example` (server SSH key, `install-ssh-key`, port 23).
- **Cloudflare R2** (10 GB free): create a private bucket `glot-backups` (R2 > Create bucket, no public access, no custom domain), then R2 > *Manage API tokens* > *Create Account API token*, permission *Object Read & Write*, *Apply to specific buckets only* = `glot-backups`. Fill `deploy/offsite/r2.env.example` with the Access Key ID, Secret and endpoint.

Then `chmod 600 deploy/.env` and test with `ssh glot@SERVER_IP glot/deploy/backup.sh`: the log ends in `off-site ok`.

### Restore drill

On the 1st of each month at 04:47 UTC, cron runs `restore.sh --drill` (log `~/backups/drill.log`): the newest local dump is restored into a throwaway `postgres:17` container, the key tables must exist, the row count of every table is printed next to the live one, the container is deleted. A failure alerts like a failed backup. To drill the encrypted off-site copy, from your Mac (the key goes through SSH stdin, held in memory, never written on the server):

```sh
ssh glot@SERVER_IP 'glot/deploy/restore.sh --drill remote' < ~/.config/glot/backup-age.key
```

### Restore

`restore.sh SOURCE` stops server and web, **replaces** the database, restarts them. SOURCE is a local `.dump`, a local `.tar.gz.age`, `remote` (newest in `daily/`) or `remote:monthly/glot-….tar.gz.age`:

```sh
ssh glot@SERVER_IP glot/deploy/restore.sh ~/backups/glot-20261018T031700Z.dump
ssh glot@SERVER_IP glot/deploy/restore.sh remote < ~/.config/glot/backup-age.key
```

From a bundle, the old `.env` and `volumes.tar` are left in `~/backups/restored-<UTC>/`.

### Total loss of the VPS

1. New server: sections 1 to 4 (bootstrap, DNS to the new IP).
2. On your Mac, fetch and decrypt the last bundle (install rclone, put the same `RCLONE_CONFIG_*` lines in your shell environment, or download it from the Cloudflare / Hetzner console):
   ```sh
   rclone copy r2:glot-backups/daily/ . --max-age 2d   # or storagebox:glot/daily/
   age -d -i ~/.config/glot/backup-age.key glot-<UTC>.tar.gz.age | tar -xzf -   # glot.dump, env, volumes.tar
   scp env glot@NEW_IP:glot-env && ssh glot@NEW_IP 'mkdir -p glot/deploy && install -m 600 glot-env glot/deploy/.env && rm glot-env'
   ```
   The old `.env` brings back `GLOT_SECRET_KEY`, the database password and the off-site settings.
3. Optional, to keep the certificates: `ssh glot@NEW_IP 'docker run --rm -i -v glot_caddy_data:/v/caddy_data -v glot_caddy_config:/v/caddy_config --entrypoint tar caddy:2 -C /v -xf -' < volumes.tar`
4. `GLOT_HOST=NEW_IP deploy/deploy.sh`, then `scp glot.dump glot@NEW_IP:backups/` and `ssh glot@NEW_IP glot/deploy/restore.sh backups/glot.dump`.

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
