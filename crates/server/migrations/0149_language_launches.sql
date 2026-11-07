-- Language launch measurement (crates/server/src/launches.rs): did adding a language move conversions?
-- `holdout`: a language experiment (insight_experiments) whose control arm is *held* on `hold_locale` (the fallback)
-- while the test arm gets the new locale; rolled out to 100 % at the end. `before_after`: same audience, N days
-- before vs after (correlation, not causation). Aggregates only, as everywhere in Insights.
ALTER TABLE insight_experiments ADD COLUMN hold_locale text;

CREATE TABLE language_launches (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    mode text NOT NULL CHECK (mode IN ('holdout', 'before_after')),
    experiment_id uuid REFERENCES insight_experiments(id) ON DELETE SET NULL,
    fallback text NOT NULL,                 -- what held-out visitors (and the "before" period) were served
    goal text NOT NULL,
    holdout_pct int NOT NULL CHECK (holdout_pct BETWEEN 0 AND 50),
    days int NOT NULL CHECK (days BETWEEN 7 AND 90),
    min_n int NOT NULL CHECK (min_n >= 50),
    launched_at timestamptz NOT NULL DEFAULT now(),
    ends_at timestamptz NOT NULL,
    status text NOT NULL DEFAULT 'measuring' CHECK (status IN ('measuring', 'rolled_out', 'invalid', 'done')),
    verdict text,                           -- frozen when the measurement ends
    looks int NOT NULL DEFAULT 0,           -- decision looks spent (alpha spending, experiments::stats)
    decided_at timestamptz,
    baseline jsonb NOT NULL,                -- {visits, conversions} of the audience in the N days before
    cost_usd double precision NOT NULL DEFAULT 0, -- translation jobs for this locale at creation, else the dry-run estimate
    cost_estimated boolean NOT NULL DEFAULT false,
    recommendation jsonb,                   -- glot's language ranking for this locale at launch, if any
    share_nonce text,                       -- NULL = not shared / revoked; the public token is an HMAC over it
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX language_launches_project ON language_launches (project_id, launched_at DESC);
CREATE UNIQUE INDEX language_launches_measuring ON language_launches (project_id, locale) WHERE status = 'measuring';
