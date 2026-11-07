-- glot Edge dashboard: DNS check results of a domain, and the proxy's daily counters (edge/metrics.rs).
ALTER TABLE edge_domains
    ADD COLUMN checks jsonb,            -- {"txt": {ok, expected, found}, "cname": {…}} of the last verification
    ADD COLUMN checked_at timestamptz;

CREATE TABLE edge_metrics_daily (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,                  -- UTC
    requests bigint NOT NULL DEFAULT 0,
    hits bigint NOT NULL DEFAULT 0,     -- translated without a translate pass (page cache or same origin body)
    translated bigint NOT NULL DEFAULT 0,
    fail_open bigint NOT NULL DEFAULT 0,
    glot_hist int8[] NOT NULL,          -- latency histograms, fixed ms buckets (metrics::BOUNDS)
    origin_hist int8[] NOT NULL,
    PRIMARY KEY (project_id, day)
);
