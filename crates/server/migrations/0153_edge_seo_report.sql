-- glot Edge international SEO report (crates/server/src/edge/seo_report.rs): the last crawl of each project
-- (raw HTML per locale, canonical, hreflang reciprocity, metadata, untranslated segments, initial vs rendered).
-- "Effectively indexed" is never stored here: it is unknown until Search Console is connected.
CREATE TABLE edge_seo_reports (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    report jsonb NOT NULL
);
