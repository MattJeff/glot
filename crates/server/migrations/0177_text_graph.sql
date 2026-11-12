-- Text graph (crates/server/src/text_graph.rs): one node per sentence a company owns, one edge per place it appears.

-- A sentence: normalized source text + context. `id` is deterministic (hash of source locale, normalized text and
-- context), so it survives key renames, refactors and a rebuild of this table.
CREATE TABLE text_anchors (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    id text NOT NULL,
    -- Placeholder-aware normalized text (`Hello {name}` = `Hello %s`): the identity, together with `context`.
    norm_hash text NOT NULL,
    -- Case and punctuation ignored: near-duplicate bucket (suggested, never merged).
    loose_hash text NOT NULL,
    -- '' = any context, 'action' = short text on a button or link, 'x:…' = given by an importer.
    context text NOT NULL DEFAULT '',
    source_text text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, id)
);
CREATE INDEX text_anchors_norm ON text_anchors (project_id, norm_hash);
CREATE INDEX text_anchors_loose ON text_anchors (project_id, loose_hash);

-- A place where a sentence appears. `surface` = catalog | connector | edge (mirrored from their own tables by
-- `sync`) or the name an importer chose (email, figma, video…); `locator` = its stable address in that surface.
CREATE TABLE text_occurrences (
    project_id uuid NOT NULL,
    surface text NOT NULL,
    locator text NOT NULL,
    anchor_id text NOT NULL,
    exact_hash text NOT NULL,
    source_text text NOT NULL,
    role text NOT NULL DEFAULT '',
    meta jsonb NOT NULL DEFAULT '{}',
    -- The sentence this place showed before its source was reworded.
    prev_anchor_id text,
    moved_at timestamptz,
    seen_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, surface, locator),
    FOREIGN KEY (project_id, anchor_id) REFERENCES text_anchors ON DELETE CASCADE
);
CREATE INDEX text_occurrences_anchor ON text_occurrences (project_id, anchor_id);

-- Translations of imported surfaces only: catalog, connector and Edge translations stay in their own tables and are
-- read live.
CREATE TABLE text_translations (
    project_id uuid NOT NULL,
    surface text NOT NULL,
    locator text NOT NULL,
    locale text NOT NULL,
    value text NOT NULL,
    status text NOT NULL CHECK (status IN ('machine', 'reviewed')),
    origin text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, surface, locator, locale),
    FOREIGN KEY (project_id, surface, locator) REFERENCES text_occurrences ON DELETE CASCADE
);

CREATE TABLE text_graph_state (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    synced_at timestamptz NOT NULL
);
