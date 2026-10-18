-- Remote MCP (POST /mcp) + its OAuth 2.1 authorization server. Every secret is stored as sha256 only.

-- Dynamic Client Registration (RFC 7591): public clients only (PKCE, no secret).
CREATE TABLE oauth_clients (
    id text PRIMARY KEY,
    name text NOT NULL,
    redirect_uris text[] NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Authorization codes: single use (deleted on exchange), 5 minutes.
CREATE TABLE oauth_codes (
    code_hash text PRIMARY KEY,
    client_id text NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scopes text[] NOT NULL,
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL,
    resource text,
    expires_at timestamptz NOT NULL
);

-- One row per consent: the current access + refresh token pair (refresh rotates both). Revocation = DELETE.
CREATE TABLE oauth_grants (
    id uuid PRIMARY KEY,
    client_id text NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scopes text[] NOT NULL,
    resource text,
    access_hash text NOT NULL UNIQUE,
    access_expires_at timestamptz NOT NULL,
    refresh_hash text NOT NULL UNIQUE,
    refresh_expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);

-- Per-token per-minute budget for /mcp, same scheme as insight_rate (one reused row per token hash).
CREATE TABLE mcp_rate (
    token_hash text PRIMARY KEY,
    window_start timestamptz NOT NULL,
    hits integer NOT NULL
);

-- Audit log of tool calls (name + outcome only: arguments may hold user copy).
CREATE TABLE mcp_audit (
    id bigserial PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    client_id text,                   -- NULL = API token
    tool text NOT NULL,
    ok boolean NOT NULL,
    ms integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX mcp_audit_project ON mcp_audit (project_id, created_at DESC);
