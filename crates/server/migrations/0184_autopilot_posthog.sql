-- Autopilot PostHog importer (crates/server/src/autopilot_posthog.rs): read-only, aggregates only.
-- The personal API key is stored encrypted (auth::encrypt) and never returned.
CREATE TABLE autopilot_posthog (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    host text NOT NULL,                     -- API host: https://us.posthog.com, https://eu.posthog.com or self-hosted
    posthog_project text NOT NULL,          -- PostHog project ID
    api_key_enc bytea NOT NULL,             -- personal API key (phx_…), scope query:read
    conversion_event text,                  -- PostHog event counted as the goal; NULL = visits only
    default_locale text,                    -- locale served when the URL carries no locale code; NULL = unknown
    backfill_days int NOT NULL DEFAULT 90 CHECK (backfill_days BETWEEN 7 AND 365),
    synced_from date,
    synced_to date,                         -- last day fetched (re-fetched next time: it was partial)
    last_sync_at timestamptz,
    last_error text,                        -- why the last sync failed, in plain words
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Same shape as insight_daily (minus the experiment arm), kept apart so Insights and launch measurement only ever
-- count the glot tag. event: $view = sessions, $pageview, $bounce = one-page sessions, $visitor = distinct visitors
-- (counted by PostHog), $conversion = sessions that fired the conversion event.
CREATE TABLE autopilot_posthog_daily (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    preferred text NOT NULL,                -- browser language, primary subtag ('und' = unknown)
    served text NOT NULL,                   -- project locale found in the path or host, else the default, else ''
    country text NOT NULL,                  -- ISO 3166 from PostHog GeoIP, '' = unknown
    event text NOT NULL,
    n bigint NOT NULL,
    PRIMARY KEY (project_id, day, preferred, served, country, event)
);
