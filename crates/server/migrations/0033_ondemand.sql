-- On-demand locales: `GET /cdn/{slug}/{locale}.json?ondemand=1` translates a locale the project doesn't ship yet.
-- The jobs themselves carry `params.ondemand_locale`, so no extra table: readiness, budget and rate are job queries.
-- ondemand_locales: whitelist, '{*}' = any locale, '{}' = off (default).
-- ondemand_budget_usd: monthly cap on what on-demand jobs may spend; NULL = no cap beyond llm.budget_usd per job.
-- ondemand_per_day: on-demand jobs a project may start per rolling 24 h (anti-abuse: the CDN URL is public).
ALTER TABLE projects
    ADD COLUMN ondemand_locales text[] NOT NULL DEFAULT '{}',
    ADD COLUMN ondemand_budget_usd double precision CHECK (ondemand_budget_usd >= 0),
    ADD COLUMN ondemand_per_day integer NOT NULL DEFAULT 5 CHECK (ondemand_per_day >= 0);
CREATE INDEX jobs_ondemand ON jobs (project_id, (params->>'ondemand_locale')) WHERE params ? 'ondemand_locale';
