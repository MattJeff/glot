-- Shared rate limit for POST /api/insights/{key}, correct across replicas (no Redis: Postgres is the only dependency).
-- One row per project, reused across windows: the counter resets when a newer minute arrives, so the table
-- never grows beyond one row per project (and goes away with it).
CREATE TABLE insight_rate (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    window_start timestamptz NOT NULL,
    hits integer NOT NULL
);

-- Hits per minute for this project's public key; NULL = server default (GLOT_INSIGHTS_RATE_PER_MIN, else 1200).
ALTER TABLE projects ADD COLUMN insights_rate_per_min integer CHECK (insights_rate_per_min > 0);
