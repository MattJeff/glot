-- Autopilot (crates/server/src/autopilot.rs): "revenue per market" on top of Insights, launches and copy
-- experiments. Aggregates only, as everywhere in Insights: no visitor, session, IP, user agent or route.

-- Per-project defaults. `value_per_conversion` is declared by the customer; NULL = use the measured average
-- (autopilot_revenue_daily) or report conversions only.
CREATE TABLE autopilot_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    goal text NOT NULL DEFAULT 'signup',
    value_per_conversion double precision CHECK (value_per_conversion > 0),
    currency text NOT NULL DEFAULT 'USD',
    mode text NOT NULL DEFAULT 'approval' CHECK (mode IN ('approval', 'auto')),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Revenue sent with conversion events (`track("purchase", {value, currency})`): one counter and one sum per
-- (day, event, currency). Deliberately not split by language, country or arm.
CREATE TABLE autopilot_revenue_daily (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    event text NOT NULL,
    currency text NOT NULL,
    n bigint NOT NULL DEFAULT 0,
    value double precision NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, day, event, currency)
);

-- One-click "open this market": the locale joins the targets, a translation job runs, then the measurement
-- (language_launches) starts once the job is done.
CREATE TABLE autopilot_openings (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    mode text NOT NULL CHECK (mode IN ('holdout', 'before_after')),
    days int NOT NULL CHECK (days BETWEEN 7 AND 90),
    goal text NOT NULL,
    status text NOT NULL DEFAULT 'translating' CHECK (status IN ('translating', 'measuring', 'failed')),
    job_id uuid REFERENCES jobs(id) ON DELETE SET NULL,
    launch_id uuid REFERENCES language_launches(id) ON DELETE SET NULL,
    estimate jsonb,                         -- the opportunity as shown when the button was clicked (prediction)
    error text,                             -- why it failed, in plain words
    created_by uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX autopilot_openings_project ON autopilot_openings (project_id, created_at DESC);
CREATE UNIQUE INDEX autopilot_openings_live ON autopilot_openings (project_id, locale) WHERE status = 'translating';

-- A copy experiment (variant_experiments) driven by Autopilot: harmful arms are withdrawn, the winner is kept
-- (auto) or proposed (approval), and it stops by itself when nothing differs.
CREATE TABLE autopilot_optimizations (
    experiment_id uuid PRIMARY KEY REFERENCES variant_experiments(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind text NOT NULL,                     -- headline | cta | pricing | other
    mode text NOT NULL CHECK (mode IN ('approval', 'auto')),
    status text NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'awaiting_approval', 'promoted', 'rolled_back', 'no_winner', 'stopped')),
    reason text,                            -- machine code, translated by the dashboard
    max_days int NOT NULL DEFAULT 30 CHECK (max_days BETWEEN 1 AND 180),
    harm_looks int NOT NULL DEFAULT 0,      -- harm analyses spent (error budget split over looks)
    dropped jsonb NOT NULL DEFAULT '[]',    -- arms withdrawn or refused: [{arm, value, views, conversions, why}]
    result jsonb,                           -- frozen report when it ends
    created_by uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    decided_at timestamptz
);
CREATE INDEX autopilot_optimizations_project ON autopilot_optimizations (project_id, created_at DESC);
