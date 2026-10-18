-- Locale insights (@glot/insights): demand gap, funnel per locale, language experiments.
-- Privacy: only daily aggregates are stored — no IP, user agent, visitor id or route.
CREATE TABLE insight_keys (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    public_key text NOT NULL UNIQUE,
    rotated_at timestamptz NOT NULL DEFAULT now()
);

-- event = '$view' for a visit (sent once per browser session), else the tracked event name.
-- country = '' when unknown, arm = '' outside an experiment ('test' | 'control').
CREATE TABLE insight_daily (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    preferred text NOT NULL,
    served text NOT NULL,
    country text NOT NULL,
    arm text NOT NULL,
    event text NOT NULL,
    n bigint NOT NULL,
    PRIMARY KEY (project_id, day, preferred, served, country, arm, event)
);

CREATE TABLE insight_experiments (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    traffic_pct int NOT NULL,
    goal text NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    ends_at timestamptz NOT NULL,
    status text NOT NULL DEFAULT 'running',
    decided_at timestamptz
);
CREATE UNIQUE INDEX insight_experiments_running ON insight_experiments (project_id, locale) WHERE status = 'running';
