-- Render protocol (crates/server/src/render_protocol.rs): `POST /v1/render` localizes any piece of content
-- (text, HTML email, Markdown, ICU template, JSON, subject + body) for a recipient locale.

-- Per-project machine keys. Only the SHA-256 of the key is stored; `prefix` is what the dashboard shows.
CREATE TABLE render_keys (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name text NOT NULL,
    prefix text NOT NULL,
    key_hash text NOT NULL UNIQUE,
    -- 'render' = may translate new content (spends the project's LLM budget);
    -- 'render:cached' = only serves what is already in memory.
    scopes text[] NOT NULL,
    monthly_limit bigint,               -- requests per calendar month (UTC); NULL = the plan's limit only
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);
CREATE INDEX render_keys_project ON render_keys (project_id);

-- The memory: one translated unit (paragraph, HTML segment, ICU message) per (project, locale, hash).
-- `source` is the masked template sent for translation: variables, numbers, emails and URLs are
-- placeholders in it, never values.
CREATE TABLE render_tm (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    hash text NOT NULL,
    source text NOT NULL,
    value text NOT NULL,
    origin text NOT NULL,               -- machine | catalog | edge
    model text,
    hits bigint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_hit_at timestamptz,
    PRIMARY KEY (project_id, locale, hash)
);

-- Counters per (project, day, key). key_id = nil uuid for the dashboard playground. No content, no variables.
CREATE TABLE render_usage (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    key_id uuid NOT NULL,
    requests bigint NOT NULL DEFAULT 0,
    units bigint NOT NULL DEFAULT 0,
    cached bigint NOT NULL DEFAULT 0,
    translated bigint NOT NULL DEFAULT 0,
    fallbacks bigint NOT NULL DEFAULT 0, -- documents returned in the source language
    cost_usd double precision NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, day, key_id)
);

-- Idempotency-Key replays (24 h). `response` holds the localized templates before variable substitution:
-- the variables of the retried request are applied again, so no personal data is stored here.
CREATE TABLE render_idempotency (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key text NOT NULL,
    request_hash text NOT NULL,
    response jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, key)
);
