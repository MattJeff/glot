-- Two-way translation (crates/server/src/two_way): what customers write back, read in the team's language,
-- answered in the customer's. Originals are always kept next to their translation.
CREATE TABLE two_way_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    -- NULL = the project's source locale.
    team_locale text,
    -- suggest: glot never sends. one_click: a person sends each reply. auto: sent unless flagged for review.
    mode text NOT NULL DEFAULT 'suggest' CHECK (mode IN ('suggest', 'one_click', 'auto')),
    -- Outbound replies whose back-translation agrees less than this with what the agent wrote wait for a person.
    review_below real NOT NULL DEFAULT 0.6,
    -- Local language detection under this confidence falls back to the thread's language, then to the hint.
    min_detect_confidence real NOT NULL DEFAULT 0.5,
    retention_days int NOT NULL DEFAULT 90,
    -- Generic channel: HMAC secret of the inbound webhook (encrypted), optional URL replies are delivered to.
    webhook_secret_enc bytea,
    reply_url text,
    -- Contact-form helper (public endpoint): off by default, stores nothing but a language counter unless form_store.
    form_enabled boolean NOT NULL DEFAULT false,
    form_store boolean NOT NULL DEFAULT false,
    form_per_day int NOT NULL DEFAULT 200,
    -- Inbound email: translated messages are forwarded to this team address.
    forward_to text,
    -- Help Scout Mailbox API 2.0 (OAuth2 client credentials) + webhook secret, encrypted.
    helpscout_app_id text,
    helpscout_app_secret_enc bytea,
    helpscout_webhook_secret_enc bytea,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE two_way_conversations (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    channel text NOT NULL CHECK (channel IN ('api', 'helpscout', 'form', 'email')),
    -- The other tool's id for the thread (an opaque string: glot never needs the customer's address).
    external_id text NOT NULL,
    customer_locale text,
    customer_name text,
    subject text,
    -- Channel data needed to answer (Help Scout customer id). No email address.
    meta jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    last_message_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, channel, external_id)
);
CREATE INDEX two_way_conversations_recent ON two_way_conversations (project_id, last_message_at DESC);

CREATE TABLE two_way_messages (
    id uuid PRIMARY KEY,
    conversation_id uuid NOT NULL REFERENCES two_way_conversations(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    direction text NOT NULL CHECK (direction IN ('in', 'out')),
    external_id text,
    original text NOT NULL,
    original_locale text,
    translation text,
    translation_locale text,
    -- Outbound only: the translation translated back into the team's language, and how much it agrees (0-1).
    back_translation text,
    confidence real,
    detect_confidence real,
    -- language_uncertain, mixed:<locales>, low_confidence, validation:<code>, pii_lost, untranslated:<why>
    flags text[] NOT NULL DEFAULT '{}',
    pii_masked int NOT NULL DEFAULT 0,
    -- in: received. out: draft | needs_review | sent | failed.
    status text NOT NULL CHECK (status IN ('received', 'draft', 'needs_review', 'sent', 'failed')),
    delivered_via text,
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz,
    UNIQUE (conversation_id, external_id)
);
CREATE INDEX two_way_messages_thread ON two_way_messages (conversation_id, created_at);

-- Which languages customers write in: a counter per day, kept when messages are purged and for forms
-- that store nothing else.
CREATE TABLE two_way_lang_stats (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day date NOT NULL,
    channel text NOT NULL,
    locale text NOT NULL,
    messages int NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, day, channel, locale)
);
