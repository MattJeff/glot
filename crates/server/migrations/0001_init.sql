CREATE TABLE users (
    id uuid PRIMARY KEY,
    email text NOT NULL UNIQUE,
    name text NOT NULL,
    password_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);

CREATE TABLE api_tokens (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name text NOT NULL,
    token_hash text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);

CREATE TABLE projects (
    id uuid PRIMARY KEY,
    slug text NOT NULL UNIQUE,
    name text NOT NULL,
    source_locale text NOT NULL,
    target_locales text[] NOT NULL,
    style text,
    public_cdn boolean NOT NULL DEFAULT false,
    llm jsonb NOT NULL,
    api_key_enc bytea,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE project_members (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role text NOT NULL CHECK (role IN ('owner', 'admin', 'translator')),
    PRIMARY KEY (project_id, user_id)
);

CREATE TABLE keys (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key text NOT NULL,
    source text NOT NULL,
    description text,
    refs jsonb NOT NULL DEFAULT '[]',
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, key)
);

CREATE TABLE translations (
    key_id uuid NOT NULL REFERENCES keys(id) ON DELETE CASCADE,
    locale text NOT NULL,
    value text NOT NULL,
    status text NOT NULL CHECK (status IN ('machine', 'reviewed', 'stale')),
    score smallint,
    issues jsonb NOT NULL DEFAULT '[]',
    origin text NOT NULL CHECK (origin IN ('machine', 'human', 'imported')),
    model text,
    source_hash text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (key_id, locale)
);

CREATE TABLE translation_history (
    id bigserial PRIMARY KEY,
    key_id uuid NOT NULL REFERENCES keys(id) ON DELETE CASCADE,
    locale text NOT NULL,
    value text NOT NULL,
    origin text NOT NULL,
    model text,
    score smallint,
    author text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX translation_history_key ON translation_history (key_id, created_at DESC);

CREATE TABLE glossary_terms (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    term text NOT NULL,
    description text,
    do_not_translate boolean NOT NULL DEFAULT false,
    case_sensitive boolean NOT NULL DEFAULT false,
    translations jsonb NOT NULL DEFAULT '{}',
    forbidden jsonb NOT NULL DEFAULT '{}'
);
CREATE UNIQUE INDEX glossary_terms_unique ON glossary_terms (project_id, lower(term));

CREATE TABLE jobs (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind text NOT NULL,
    status text NOT NULL CHECK (status IN ('queued', 'running', 'done', 'failed')),
    params jsonb NOT NULL DEFAULT '{}',
    progress_done int NOT NULL DEFAULT 0,
    progress_total int NOT NULL DEFAULT 0,
    cost_usd double precision NOT NULL DEFAULT 0,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz
);
CREATE INDEX jobs_queued ON jobs (created_at) WHERE status = 'queued';
CREATE INDEX jobs_project ON jobs (project_id, created_at DESC);
