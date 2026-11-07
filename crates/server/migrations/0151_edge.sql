-- glot Edge (crates/server/src/edge, glot_core::edge): HTML translated by segments, without i18n keys.
-- Segment id: `hash` = sha256(source_locale || '\0' || normalized text) truncated to 16 bytes (32 hex);
-- `context` = '<role>|<page template>' (glot_core::edge::tm::context): "Save" on a button and in a price row
-- are two rows. Translations are looked up by exact context, else by text when only one translation exists.
-- No visitor data anywhere: segment texts are page copy, URLs are stored as paths without query string.

-- Per-project Edge settings; no row = defaults (edge::discover::Settings::load).
CREATE TABLE edge_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    enabled boolean NOT NULL DEFAULT true,            -- kill switch: false = pure passthrough to the origin
    auto_translate boolean NOT NULL DEFAULT true,     -- missing segments queue an `edge_translate` job
    discover_include text[] NOT NULL DEFAULT '{}',    -- path globs where discovery is allowed; {} = every public path
    discover_exclude text[] NOT NULL DEFAULT '{}',    -- path globs never discovered
    selectors_include text[] NOT NULL DEFAULT '{}',   -- CSS selectors limiting extraction (worker/proxy/patcher)
    selectors_exclude text[] NOT NULL DEFAULT '{}',   -- CSS selectors never extracted (user content, widgets)
    critical_patterns text[] NOT NULL DEFAULT '{/,/pricing*,/signup*,/register*,/checkout*,/pay*}',
    sensitive_patterns text[] NOT NULL DEFAULT '{/checkout*,/pay*,/payment*,/billing*,/account*,/settings*}',
    critical_threshold real NOT NULL DEFAULT 0.98 CHECK (critical_threshold BETWEEN 0 AND 1),
    daily_cap int NOT NULL DEFAULT 2000 CHECK (daily_cap BETWEEN 0 AND 100000), -- new segments accepted per 24 h
    monthly_budget_usd double precision CHECK (monthly_budget_usd >= 0),        -- auto-translation spend cap
    rendering text NOT NULL DEFAULT 'auto' CHECK (rendering IN ('origin', 'prerender', 'auto')),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Source segments seen in production (aggregated, deduplicated).
CREATE TABLE edge_segments (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    hash text NOT NULL,
    context text NOT NULL DEFAULT '',
    source_text text NOT NULL,
    needs_review boolean NOT NULL DEFAULT false,      -- first seen on a sensitive page: translations start 'pending'
    first_seen timestamptz NOT NULL DEFAULT now(),
    last_seen timestamptz NOT NULL DEFAULT now(),
    hits bigint NOT NULL DEFAULT 1,
    url_sample text,                                  -- a path where it was seen (no query string)
    PRIMARY KEY (project_id, hash, context)
);
CREATE INDEX edge_segments_first_seen ON edge_segments (project_id, first_seen DESC);

CREATE TABLE edge_translations (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    hash text NOT NULL,
    context text NOT NULL DEFAULT '',
    locale text NOT NULL,
    value text NOT NULL,
    -- pending = awaiting review (sensitive page), never served
    status text NOT NULL CHECK (status IN ('machine', 'reviewed', 'stale', 'pending')),
    origin text NOT NULL DEFAULT 'machine',           -- machine | human
    model text,
    score smallint,
    issues jsonb NOT NULL DEFAULT '[]',
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, hash, context, locale),
    FOREIGN KEY (project_id, hash, context) REFERENCES edge_segments ON DELETE CASCADE
);
CREATE INDEX edge_translations_locale ON edge_translations (project_id, locale);

-- Which segments a page renders (per-URL / critical-page coverage). Paths without query string.
CREATE TABLE edge_page_segments (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path text NOT NULL,
    hash text NOT NULL,
    context text NOT NULL DEFAULT '',
    last_seen timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, path, hash, context)
);

CREATE TABLE edge_domains (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    host text NOT NULL UNIQUE,                        -- lowercase, no port: es.cliponit.app
    mode text NOT NULL CHECK (mode IN ('subdomain', 'subdirectory', 'middleware')),
    locale text NOT NULL,
    origin_url text NOT NULL,                         -- fixed origin for this host (never taken from the request)
    verify_token text NOT NULL,
    verified_at timestamptz,
    enabled boolean NOT NULL DEFAULT true,            -- per-domain kill switch
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX edge_domains_project ON edge_domains (project_id);

-- One monotonic version per (project, locale), shared by the CDN catalogs, the edge TM, pre-rendered snapshots
-- and the client patcher. Bumped by triggers on every write to `translations` and `edge_translations`.
CREATE TABLE edge_catalog_version (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    version bigint NOT NULL DEFAULT 1,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, locale)
);

-- Per-minute budget of `POST /edge/v1/{project}/missing` (same fixed-window scheme as feedback_rate).
CREATE TABLE edge_missing_rate (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    window_start timestamptz NOT NULL,
    hits int NOT NULL
);

CREATE FUNCTION edge_bump_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_TABLE_NAME = 'translations' THEN
        INSERT INTO edge_catalog_version AS v (project_id, locale)
        SELECT DISTINCT k.project_id, c.locale FROM changed c JOIN keys k ON k.id = c.key_id
        ON CONFLICT (project_id, locale) DO UPDATE SET version = v.version + 1, updated_at = now();
    ELSE
        INSERT INTO edge_catalog_version AS v (project_id, locale)
        SELECT DISTINCT c.project_id, c.locale FROM changed c
        ON CONFLICT (project_id, locale) DO UPDATE SET version = v.version + 1, updated_at = now();
    END IF;
    RETURN NULL;
END $$;

-- Statement-level: a bulk push bumps each (project, locale) once per statement.
CREATE TRIGGER translations_version_ins AFTER INSERT ON translations
    REFERENCING NEW TABLE AS changed FOR EACH STATEMENT EXECUTE FUNCTION edge_bump_version();
CREATE TRIGGER translations_version_upd AFTER UPDATE ON translations
    REFERENCING NEW TABLE AS changed FOR EACH STATEMENT EXECUTE FUNCTION edge_bump_version();
CREATE TRIGGER translations_version_del AFTER DELETE ON translations
    REFERENCING OLD TABLE AS changed FOR EACH STATEMENT EXECUTE FUNCTION edge_bump_version();
CREATE TRIGGER edge_translations_version_ins AFTER INSERT ON edge_translations
    REFERENCING NEW TABLE AS changed FOR EACH STATEMENT EXECUTE FUNCTION edge_bump_version();
CREATE TRIGGER edge_translations_version_upd AFTER UPDATE ON edge_translations
    REFERENCING NEW TABLE AS changed FOR EACH STATEMENT EXECUTE FUNCTION edge_bump_version();
CREATE TRIGGER edge_translations_version_del AFTER DELETE ON edge_translations
    REFERENCING OLD TABLE AS changed FOR EACH STATEMENT EXECUTE FUNCTION edge_bump_version();
