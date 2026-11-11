-- CMS connectors (crates/server/src/connectors): one account per connected site/store/space, its mirrored items.
CREATE TABLE connector_accounts (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind text NOT NULL,
    name text NOT NULL,
    -- AES-GCM (crate::auth::encrypt) of the credentials JSON; never logged, never sent to the browser.
    creds_enc bytea NOT NULL,
    settings jsonb NOT NULL DEFAULT '{}',
    webhook_secret text NOT NULL,
    -- idle | queued | running | ok | error
    status text NOT NULL DEFAULT 'idle' CHECK (status IN ('idle', 'queued', 'running', 'ok', 'error')),
    -- Pull resume point of the sync in progress (pagination cursor), NULL = start / none.
    sync_cursor text,
    -- Incremental (webhook) sync: only content changed since this instant; NULL = full pull.
    sync_since timestamptz,
    sync_started_at timestamptz,
    -- A webhook arrived while running: sync again afterwards.
    dirty boolean NOT NULL DEFAULT false,
    -- Translation job queued by the last sync; pushed once it finishes.
    pending_job uuid,
    last_sync_at timestamptz,
    last_push_at timestamptz,
    last_error text,
    stats jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX connector_accounts_project ON connector_accounts (project_id);
CREATE INDEX connector_accounts_queued ON connector_accounts (created_at) WHERE status = 'queued';

CREATE TABLE connector_items (
    account_id uuid NOT NULL REFERENCES connector_accounts(id) ON DELETE CASCADE,
    external_id text NOT NULL,
    field text NOT NULL,
    -- glot key (`{kind}:{resource}:{id}:{field}`); HTML / rich text fields own `{key}#{hash}` segment keys.
    key text NOT NULL,
    -- plain | html | rich
    format text NOT NULL,
    -- Source value as pulled (HTML, plain text or rich text JSON): rebuilt per locale on push.
    source text NOT NULL,
    source_hash text NOT NULL,
    -- Connector-specific data needed to push back (e.g. Shopify's translatableContentDigest).
    meta jsonb NOT NULL DEFAULT '{}',
    -- Per locale: sha256 of the value last pushed.
    pushed jsonb NOT NULL DEFAULT '{}',
    pushed_at timestamptz,
    seen_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, external_id, field)
);
