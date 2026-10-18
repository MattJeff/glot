-- Visual QA reports uploaded by packages/visual-qa (`--upload`), and the locale-matrix preview settings.
CREATE TABLE visual_reports (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    label text,
    report jsonb NOT NULL,
    summary jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX visual_reports_project ON visual_reports (project_id, created_at DESC);

CREATE TABLE visual_files (
    report_id uuid NOT NULL REFERENCES visual_reports(id) ON DELETE CASCADE,
    path text NOT NULL,
    data bytea NOT NULL,
    PRIMARY KEY (report_id, path)
);

CREATE TABLE preview_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    base_url text,
    url_pattern text NOT NULL DEFAULT '/{locale}{route}',
    routes text[] NOT NULL DEFAULT '{}'
);
