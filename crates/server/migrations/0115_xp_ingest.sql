-- Retry dedup for experiment beacons (experiments/ingest.rs): a random per-beacon id, kept one hour, never linked
-- to a visitor. UNLOGGED: a cache, losing it on crash only lets a few retries count twice.
CREATE UNLOGGED TABLE xp_dedup (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    nonce text NOT NULL,
    seen_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, nonce)
);
CREATE INDEX xp_dedup_seen ON xp_dedup (seen_at);
