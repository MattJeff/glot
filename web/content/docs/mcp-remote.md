# Remote MCP (OAuth)

Lovable, Bolt, v0, ChatGPT and Claude on the web only connect to **remote** MCP servers. `glot-server` serves one at `POST /mcp` (MCP Streamable HTTP, stateless, JSON responses). Each connection works on **one project** stored on the server. It has the same tools as the [local MCP server](/docs/mcp), but they act on the server's keys, translations and glossary instead of files in a repo.

This page assumes a [self-hosted server](/docs/self-hosting) reachable over HTTPS. Below, `https://glot.example.com` is its public URL and `my-app` is a project slug.

## Server settings

| Variable | Purpose |
|---|---|
| `GLOT_PUBLIC_URL` | Public base URL, e.g. `https://glot.example.com`. It is used as the OAuth issuer and the MCP resource (`/mcp`), and it is the only `Host` `/mcp` accepts. Without it, URLs are derived from the request (fine locally, not in production). |
| `GLOT_WEB_ORIGIN` | Dashboard origin. The consent screen lives there at `/oauth/authorize`, and it is always an allowed `Origin`. |
| `GLOT_MCP_ALLOWED_ORIGINS` | Extra browser origins allowed to call `/mcp`, comma-separated (e.g. `http://localhost:6274` for MCP Inspector). A request with any other `Origin` header is refused with 403. Server-to-server clients send no `Origin` and are not affected. |
| `GLOT_MCP_RATE_PER_MIN` | Requests per minute per token (default 120). Counted in Postgres, so the limit holds across replicas. Over the limit: 429 with `Retry-After`. |

`/mcp`, `/oauth/*` and `/.well-known/*` must be reachable at `GLOT_PUBLIC_URL`. If the dashboard and the API share a domain, route these paths to `glot-server`, not to Next.js.

## Two ways to authenticate

1. **API token** (simplest, for Claude Code, Cursor and scripts). Create a token in the dashboard (Settings → API tokens) and send it as `Authorization: Bearer glot_…`. Choose the project with `?project=my-app` in the URL or an `X-Glot-Project: my-app` header. The token gets every scope, limited by your role on the project.
2. **OAuth 2.1** (for Lovable, Bolt, v0, ChatGPT and Claude). Give the client only the URL `https://glot.example.com/mcp`. The client discovers the authorization server, registers itself, and opens glot's consent screen. There you pick the project and the scopes. Your role on the project still applies: a translator cannot add keys, even with the `write` scope.

| Scope | Tools |
|---|---|
| `read` | `status`, `get_context`, `propose_string`, `check`, `glossary_lookup` |
| `translate` | `translate` (server job), `review` (back-translation). These use the project's LLM budget. |
| `write` | `add_string`, `set_translation`, `glossary_upsert`, `translate_repo` |

`tools/list` only returns the tools the token's scopes allow. Calling another tool returns `403` with `WWW-Authenticate: Bearer error="insufficient_scope", scope="…"`, so the client can ask you for more access (step-up).

## Claude Code

With an API token:

```sh
claude mcp add --transport http glot "https://glot.example.com/mcp?project=my-app" \
  --header "Authorization: Bearer $GLOT_TOKEN"
```

With OAuth (Claude Code opens the browser for consent):

```sh
claude mcp add --transport http glot https://glot.example.com/mcp
# then in Claude Code: /mcp → glot → Authenticate
```

### `.mcp.json` (project scope, HTTP)

```json
{
  "mcpServers": {
    "glot": {
      "type": "http",
      "url": "https://glot.example.com/mcp?project=my-app",
      "headers": { "Authorization": "Bearer ${GLOT_TOKEN}" }
    }
  }
}
```

Leave out `headers` and `?project=` to use OAuth instead.

## Cursor

In `~/.cursor/mcp.json` or `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "glot": {
      "url": "https://glot.example.com/mcp?project=my-app",
      "headers": { "Authorization": "Bearer glot_…" }
    }
  }
}
```

For OAuth, use `{ "url": "https://glot.example.com/mcp" }` and click **Connect** next to the server in Cursor's MCP settings. Cursor's `cursor://…` redirect URI is accepted.

## Lovable

Settings → **Connectors** → **Custom MCP** → **Add**:

- **Server name**: `glot`
- **Server URL**: `https://glot.example.com/mcp`
- **Authentication**: **OAuth** (the default). Click **Add & authorize**, sign in to glot, pick the project, then allow.

If you choose **Bearer token / API key** instead, paste a `glot_…` token and use `https://glot.example.com/mcp?project=my-app` as the URL. Lovable connections are per user and only used in chat, not in your published app.

## Bolt

In the chat box: **+** → **Connectors (MCP)** → **Manage connectors** → **Custom MCP server**:

- **URL**: `https://glot.example.com/mcp`
- **Transport**: HTTP
- **Authentication**: **MCP OAuth** (or **API key** with a `glot_…` token and `?project=my-app` in the URL)

## v0

v0 connects to custom MCP servers with OAuth. Add a custom MCP server with the URL `https://glot.example.com/mcp`. v0 reads `/.well-known/oauth-protected-resource`, registers itself through Dynamic Client Registration, and sends you to glot's consent screen.

If v0 asks for OAuth fields by hand (as the v0 Platform API does), use:

- **Authorization URL**: `https://glot.example.com/oauth/authorize`
- **Token URL**: `https://glot.example.com/oauth/token`
- **Scopes**: `read translate write`
- **Resource**: `https://glot.example.com/mcp`
- **Client ID**: register v0's redirect URI once with `curl -X POST https://glot.example.com/oauth/register -H 'content-type: application/json' -d '{"client_name":"v0","redirect_uris":["<v0 redirect URI>"]}'` and use the `client_id` it returns. There is no client secret.

## ChatGPT

Settings → **Apps & Connectors** → **Advanced settings** → turn on **Developer mode**. Then **Create** a connector:

- **MCP server URL**: `https://glot.example.com/mcp`
- **Authentication**: **OAuth**

ChatGPT registers itself and opens the glot consent screen.

## Claude (web and desktop)

Settings → **Connectors** → **Add custom connector** → URL `https://glot.example.com/mcp`. Claude handles OAuth and opens the glot consent screen.

## Tools

The tools work like the [local ones](/docs/mcp#tools), with these differences:

- `get_context {keys?, file?}`: `file` matches the usages recorded by `glot push`. The server has no copy of your code.
- `translate {locales?, keys?, force?, wait_seconds?}` queues a server job and waits up to `wait_seconds` (default 20, maximum 50), then returns the job's status, progress and cost.
- `review {locale, keys?, refresh?}` runs a back-translation review with the project's LLM.
- `translate_repo {base?}` opens a GitHub pull request on a new `glot/l10n-…` branch. The PR contains the source and every target catalog, plus `glot.lock`, exported from the server. It uses the repository, base branch, catalog path pattern and token from the project's GitHub settings (the ones the feedback loop uses). Run `translate` first. The server does not extract or rewrite code: for that, use `glot init` / `i18nize_file` with the local MCP server.
- Hosted mode has no `find_hardcoded`, `i18nize_file` or `remove_keys`, because the server cannot read or edit your files.

## OAuth endpoints

| Endpoint | Spec |
|---|---|
| `GET /.well-known/oauth-protected-resource` | RFC 9728. Its URL is also sent in `WWW-Authenticate: Bearer resource_metadata="…"` on every 401. |
| `GET /.well-known/oauth-authorization-server` | RFC 8414 metadata (S256 only, public clients). |
| `POST /oauth/register` | RFC 7591 Dynamic Client Registration. Redirect URIs must be https, loopback http, or a private-use scheme. |
| `GET /oauth/authorize` | Requires the authorization code + PKCE S256 flow and accepts `resource` (RFC 8707). It redirects to the dashboard consent page. The response carries `iss` (RFC 9207). |
| `POST /oauth/token` | `authorization_code` and `refresh_token`. Access tokens last 1 hour and refresh tokens 30 days. Each refresh rotates both tokens. |
| `POST /oauth/revoke` | RFC 7009. Revoking either token ends the whole grant. |

Codes, access tokens and refresh tokens are stored as SHA-256 hashes only. A token issued for another `resource` is refused. Removing a user from a project invalidates that user's grants on it.

## Audit

Every tool call is logged in `mcp_audit` with the user, project, OAuth client (or `NULL` for an API token), tool, outcome and duration. Arguments are never logged, because they can contain your product copy.
