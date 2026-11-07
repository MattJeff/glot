-- Launch rollout guardrails (crates/server/src/launches.rs): a holdout only goes to 100 % on its own when the
-- launch opted in (`auto_rollout`) and every condition holds (min conversions per arm, 7 days, no SRM, credible
-- lift, guardrail events not degraded). Otherwise it waits in `ready` / `done` / `invalid` for a confirmation.
ALTER TABLE language_launches
    ADD COLUMN min_conversions int NOT NULL DEFAULT 30 CHECK (min_conversions >= 1),
    ADD COLUMN guardrails jsonb NOT NULL DEFAULT '[]',   -- [{event, higher_is_worse}]
    ADD COLUMN auto_rollout boolean NOT NULL DEFAULT false;
ALTER TABLE language_launches DROP CONSTRAINT language_launches_status_check;
ALTER TABLE language_launches ADD CONSTRAINT language_launches_status_check
    CHECK (status IN ('measuring', 'ready', 'rolled_out', 'invalid', 'done'));
-- The holdout no longer lapses at the measurement end date (that was an implicit rollout): the launch stops it.
UPDATE insight_experiments SET ends_at = 'infinity'
WHERE id IN (SELECT experiment_id FROM language_launches WHERE status = 'measuring');
