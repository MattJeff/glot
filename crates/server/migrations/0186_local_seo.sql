-- Local SEO (crates/server/src/local_seo): search-facing content (title, description, headings, slug suggestion,
-- FAQ) adapted per page × market to what locals search, while the body stays a faithful translation.
-- Nothing here is served before a human approves it, unless an admin switched the project to mode 'auto'.
-- No visitor data: pages are public page copy, demand terms are aggregated search queries.

CREATE TABLE local_seo_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    mode text NOT NULL DEFAULT 'review' CHECK (mode IN ('review', 'auto')),
    site_url text,                                   -- where to read pages when neither Edge nor Preview knows the site
    mode_set_by uuid REFERENCES users(id) ON DELETE SET NULL,  -- who consented to 'auto'
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Source page as last scanned (raw HTML, fetched anonymously: no logged-in content).
CREATE TABLE local_seo_pages (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path text NOT NULL,
    content jsonb NOT NULL,                          -- local_seo::Page: title, description, h1, h2[], text excerpt
    fingerprint text NOT NULL,                       -- of title + description + h1: a change makes decisions stale
    scanned_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, path)
);

-- What locals search, per market. path '' = the whole market (manual keyword lists).
CREATE TABLE local_seo_demand (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    path text NOT NULL DEFAULT '',
    term text NOT NULL,
    source text NOT NULL,                            -- manual | gsc | gsc_site
    impressions bigint,                              -- NULL = unknown (never invented)
    clicks bigint,
    position double precision,
    fetched_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, locale, path, term)
);

CREATE TABLE local_seo_overrides (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,                            -- the market: es-ES and es-MX are two rows
    path text NOT NULL,
    -- proposed: waiting for a human; approved: served; auto: served by consent of mode 'auto';
    -- rejected: never served, blocks the language-level fallback; stale: the source page changed, not served;
    -- needs_demand: no search data for this market yet.
    status text NOT NULL CHECK (status IN ('proposed', 'approved', 'auto', 'rejected', 'stale', 'needs_demand')),
    current jsonb NOT NULL DEFAULT '{}',             -- the faithful translation at proposal time
    proposed jsonb NOT NULL DEFAULT '{}',            -- local_seo::Content from the LLM
    edited jsonb,                                    -- local_seo::Content from a human; replaces `proposed`
    terms jsonb NOT NULL DEFAULT '[]',               -- demand terms shown as the "why"
    rationale text NOT NULL DEFAULT '',
    issues jsonb NOT NULL DEFAULT '[]',
    fingerprint text NOT NULL DEFAULT '',
    model text,
    decided_by uuid REFERENCES users(id) ON DELETE SET NULL,
    decided_at timestamptz,
    applied_at timestamptz,                          -- start of the "after" window of the search metrics
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, locale, path)
);

-- Google Search Console connection (the customer's own OAuth consent, read-only scope).
CREATE TABLE local_seo_connections (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    site_url text NOT NULL,                          -- the Search Console property: sc-domain:example.com or https://example.com/
    refresh_token_enc bytea NOT NULL,                -- AES-256-GCM (GLOT_SECRET_KEY), never returned by the API
    connected_by uuid REFERENCES users(id) ON DELETE SET NULL,
    connected_at timestamptz NOT NULL DEFAULT now()
);

-- Daily search performance of a page in a market (Search Console), to compare before / after `applied_at`.
CREATE TABLE local_seo_metrics (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    path text NOT NULL,
    day date NOT NULL,
    impressions bigint NOT NULL,
    clicks bigint NOT NULL,
    position double precision NOT NULL,
    PRIMARY KEY (project_id, locale, path, day)
);
