-- glot Edge: SPA snapshots rendered in the customer's CI (`glot prerender --upload`), for projects that do not run
-- a browser on the glot server. Keyed by the page path on the origin (the CI renders a preview URL, the proxy asks
-- for the origin's path). The proxy prefers them to the renderer service; a re-import replaces them.
CREATE TABLE edge_snapshots (
    project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path        text NOT NULL,
    html        text NOT NULL,
    rendered_at timestamptz NOT NULL DEFAULT now(),
    imported_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, path)
);
