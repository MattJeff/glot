-- Layout RUM (`@glot/insights/layout`, crates/server/src/layout_rum.rs): which translated strings overflow their box
-- for real visitors. Counters per (day, key, locale, viewport band); no IP, user agent, URL, text or visitor id.
CREATE TABLE layout_rum_daily (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    key text NOT NULL,
    locale text NOT NULL,
    vw_min int NOT NULL,             -- viewport band lower bound (px), see layout_rum::BANDS
    views bigint NOT NULL DEFAULT 0, -- page views where the key rendered visibly
    overflows bigint NOT NULL DEFAULT 0,
    fallback bigint NOT NULL DEFAULT 0, -- overflows while the element's webfont had not loaded
    box_px bigint NOT NULL DEFAULT 0,   -- sums over overflows: box width / width the text needed
    need_px bigint NOT NULL DEFAULT 0,
    font text NOT NULL DEFAULT '',      -- last seen family + size, e.g. "Inter 14"
    PRIMARY KEY (project_id, day, key, locale, vw_min)
);

-- Repairs proposed by the loop: one per (key, locale, text it replaces), so a rejected text is never re-proposed.
CREATE TABLE layout_rum_fixes (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key text NOT NULL,
    locale text NOT NULL,
    previous text NOT NULL,
    proposal text,                    -- NULL: no variant passed the validator + pixel budget
    evidence text NOT NULL,
    status text NOT NULL,             -- pr | suggested | failed
    pr_url text,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, key, locale, previous)
);
