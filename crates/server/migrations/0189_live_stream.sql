-- Live stream translation (crates/server/src/live_stream.rs): translating text as it is generated or typed.
-- Off until a project admin opts in: the text is end-user content.
CREATE TABLE live_stream_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    enabled boolean NOT NULL DEFAULT false,
    -- Also accept the project's public key (pk_…) from browsers; off = server-side API tokens only.
    browser boolean NOT NULL DEFAULT false,
    -- Emails and long digit runs (phones, cards) are replaced by placeholders before the LLM provider.
    mask_pii boolean NOT NULL DEFAULT true,
    -- Keep translated units (masked text) so a repeated sentence costs nothing. Off = only hashes and counters.
    store boolean NOT NULL DEFAULT false,
    -- Faster model for this path; NULL = the project's model.
    model text,
    daily_chars integer NOT NULL DEFAULT 500000 CHECK (daily_chars >= 0),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- One row per distinct unit seen. `hash` = sha256 keyed by the server secret over (project, glossary/style
-- fingerprint, masked unit): the text cannot be recovered from it. `target` (masked translation) only when
-- `live_stream_settings.store` was on.
CREATE TABLE live_stream_units (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    hash text NOT NULL,
    target text,
    hits bigint NOT NULL DEFAULT 1,
    last_seen timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, locale, hash)
);

-- Daily counters (the daily cap and the dashboard). No text.
CREATE TABLE live_stream_usage (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    requests bigint NOT NULL DEFAULT 0,
    chars bigint NOT NULL DEFAULT 0,
    units bigint NOT NULL DEFAULT 0,
    llm_units bigint NOT NULL DEFAULT 0,
    known_units bigint NOT NULL DEFAULT 0,
    failed_units bigint NOT NULL DEFAULT 0,
    first_ms bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, day)
);
