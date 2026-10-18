# Self-hosting

> **Experimental.** The server and dashboard work (the dashboard is translated by glot in CI) but are young: expect rough edges and breaking changes between minor versions, and back up the database. The stable core is the CLI and the MCP server, which need no server at all.

The CLI and the MCP server work with no server at all: Git is the source of truth. The optional server adds a shared dashboard (keys editor, review, glossary, jobs, locale matrix, visual reports, [Insights](insights.md)), an HTTP API, and an OTA CDN for catalogs. It is the same open-source code; there is no hosted-only feature.

## Docker Compose

From a clone of the repo:

```sh
GLOT_SECRET_KEY=$(openssl rand -hex 32) docker compose --profile full up -d --build --wait
curl localhost:8790/healthz            # ok (migrations run at startup)
open http://localhost:3100             # dashboard; /api and /cdn are proxied to the server
docker compose --profile full down     # add -v to drop the database volume
```

Three services: `postgres` (17), `server` (Rust, axum) and `web` (Next.js standalone). Cold build: about 7 minutes for the server (mostly pulling the Rust image, ~1.5 min compile) and 2 for the web. Images are ~170 MB (server) and ~440 MB (web).

| Variable | Default | |
|---|---|---|
| `GLOT_SECRET_KEY` | insecure dev key | Encrypts stored LLM keys. **Set it** (32 random bytes, hex). |
| `GLOT_PG_PORT` / `GLOT_API_PORT` / `GLOT_WEB_PORT` | 5433 / 8790 / 3100 | Host ports |
| `GLOT_WEB_ORIGIN` | `http://localhost:3100` | Set it to the dashboard URL if you change the web port or put it behind a domain |
| `GLOT_API_URL` (web build arg) | `http://server:8790` | Baked into the Next build (rewrites are resolved at build time): changing it means rebuilding `web` |
| `GLOT_INSIGHTS_RATE_PER_MIN` | 1200 | Insights beacon budget per project per minute |
| `GLOT_FEEDBACK_RATE_PER_MIN` | 30 | Feedback widget budget per project per minute |
| `DATABASE_URL` | set by Compose | Postgres URL (required when running `glot-server` outside Compose) |
| `GLOT_BIND` | `127.0.0.1:8790` | Listen address of `glot-server` outside Compose |
| `GLOT_GITHUB_API` | `https://api.github.com` | GitHub API base for feedback PRs (GitHub Enterprise) |
| `RESEND_API_KEY` | empty (email off) | [Resend](https://resend.com) API key: email verification, password reset, password/email change notices, opt-in "job finished" emails. Unset: no email, no verification banner, `forgot-password` answers 503 |
| `RESEND_FROM` | `glot <onboarding@resend.dev>` (Compose prod: `glot <noreply@mail.$GLOT_DOMAIN>`) | Sender. Use an address on a domain verified in Resend, ideally a subdomain such as `mail.example.com` so your apex keeps its own reputation |
| `RESEND_REPLY_TO` | none | Reply-To header |
| `RESEND_API_URL` | `https://api.resend.com` | Resend API base (tests / proxies) |
| `GLOT_EMAIL_RATE_PER_HOUR` | 10 | Max emails per recipient per hour (all kinds), counted in Postgres |

Only Postgres needs a volume. Put the web container behind your reverse proxy with TLS; it proxies `/api` and `/cdn` to the server, so one origin is enough and the session cookie just works.

## Connect the CLI

Create an API token in the dashboard (**Settings → API tokens**), then in `glot.toml`:

```toml
[server]
url = "https://glot.example.com"
project = "my-saas"
token_env = "GLOT_TOKEN"
```

```sh
export GLOT_TOKEN=...
glot push      # catalogs, glot.lock, glossary and code context (refs, UI roles) to the server
glot pull      # back to Git, with descriptions edited in the dashboard
```

Server-side translation jobs use the project's LLM settings (provider, model, key stored encrypted, budget). With an organization-level Anthropic key, set the workspace to bill in the project settings.

## OTA catalogs

`GET /cdn/{slug}/{locale}.json` serves the nested catalog with `ETag` and `Cache-Control`, public when the project enables `public_cdn`. Useful for mobile apps or hotfixes without a deploy; Git remains the source of truth.

## Local development

```sh
docker compose up -d postgres          # postgres://glot:glot@localhost:5433/glot
cargo run -p glot-server               # API on :8790
cd web && pnpm install && pnpm dev     # dashboard on :3100
```

The OpenAPI document is at `/api/openapi.json`. Architecture and the full HTTP API: [ARCHITECTURE.md](https://github.com/MattJeff/glot/blob/main/ARCHITECTURE.md).
