-- New projects discover up to 10000 new segments per 24 h (was 2000: a site shipping a redesign hit it in a day).
-- Saved settings keep their value (a project that chose 2000 keeps it).
ALTER TABLE edge_settings ALTER COLUMN daily_cap SET DEFAULT 10000;

-- Segments refused by `edge_settings.daily_cap` (edge::missing): kept here, moved to `edge_segments` when the
-- rolling 24 h window has room again, counted in the coverage API so a full cap is never silent.
CREATE TABLE edge_segments_deferred (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    hash text NOT NULL,
    context text NOT NULL,
    source_text text NOT NULL,
    needs_review boolean NOT NULL DEFAULT false,
    url_sample text,
    refused_at timestamptz NOT NULL DEFAULT now(),   -- last refusal (re-reported = still on the site)
    PRIMARY KEY (project_id, hash, context)
);
CREATE INDEX edge_segments_deferred_refused ON edge_segments_deferred (project_id, refused_at DESC);
