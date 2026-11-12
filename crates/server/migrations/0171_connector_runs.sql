-- Connectors a merchant can trust: preview / pause (`hold`), webhook health, human edits kept, run history and rollback.
ALTER TABLE connector_accounts
    -- NULL = automatic. 'translate' = stop after the pull (preview, paused: nothing translated, nothing written).
    -- 'push' = translate but write nothing to the platform until the user publishes.
    ADD COLUMN hold text CHECK (hold IN ('translate', 'push')),
    ADD COLUMN last_webhook_at timestamptz;

-- Per locale: fingerprint (text only, markup ignored) of the value glot last wrote. A platform value with another
-- fingerprint was edited there by a human: kept, never overwritten.
ALTER TABLE connector_items ADD COLUMN written jsonb NOT NULL DEFAULT '{}';

-- One row per push that wrote something; its writes hold the previous platform values (rollback).
CREATE TABLE connector_runs (
    id uuid PRIMARY KEY,
    account_id uuid NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    started_at timestamptz NOT NULL DEFAULT now(),
    stats jsonb NOT NULL DEFAULT '{}',
    rolled_back_at timestamptz
);
CREATE INDEX connector_runs_account ON connector_runs (account_id, started_at DESC);

CREATE TABLE connector_writes (
    run_id uuid NOT NULL REFERENCES connector_runs(id) ON DELETE CASCADE,
    external_id text NOT NULL,
    field text NOT NULL,
    locale text NOT NULL,
    -- Platform value before the write; NULL = there was no translation (or the connector cannot read them).
    previous text,
    PRIMARY KEY (run_id, external_id, field, locale)
);
