-- Copy experiments: per (key, locale), validated LLM variants of a high-stakes string compete on conversion
-- (Thompson sampling). Arm 0 = the official translation at creation. Only aggregate counters are stored.
CREATE TABLE variant_experiments (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key text NOT NULL,
    locale text NOT NULL,
    -- source text when the variants were generated: a changed source stops serving them.
    source text NOT NULL,
    goal text NOT NULL,
    -- share of visitors (1-100) that enter the experiment; the rest see the official translation.
    explore_pct int NOT NULL,
    -- minimum visitors per arm before a winner may be declared.
    min_n int NOT NULL,
    -- interim analyses done (one each time traffic doubles past the burn-in; error budget spent over looks).
    looks int NOT NULL DEFAULT 0,
    -- running | decided (winner served to the explored traffic, awaiting promotion) | promoted | stopped
    status text NOT NULL DEFAULT 'running',
    winner int,
    created_at timestamptz NOT NULL DEFAULT now(),
    decided_at timestamptz
);
CREATE UNIQUE INDEX variant_experiments_live ON variant_experiments (project_id, key, locale)
    WHERE status IN ('running', 'decided');
CREATE INDEX variant_experiments_locale ON variant_experiments (project_id, locale) WHERE status IN ('running', 'decided');

-- Only variants that passed validation are ever inserted (never served otherwise).
CREATE TABLE variant_arms (
    experiment_id uuid NOT NULL REFERENCES variant_experiments(id) ON DELETE CASCADE,
    arm int NOT NULL,
    value text NOT NULL,
    tone text NOT NULL,
    views bigint NOT NULL DEFAULT 0,
    conversions bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (experiment_id, arm)
);
