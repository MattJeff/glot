-- Transactional email (crates/server/src/email.rs): verification, password reset, notices.
ALTER TABLE users
    ADD COLUMN email_verified_at timestamptz,
    ADD COLUMN locale text NOT NULL DEFAULT 'en',
    ADD COLUMN notify_job_done boolean NOT NULL DEFAULT false;

-- Who started a job (dashboard/API): receives the opt-in "job finished" email.
ALTER TABLE jobs ADD COLUMN created_by uuid REFERENCES users(id) ON DELETE SET NULL;

-- Single-use links. Only the sha256 of the token is stored.
-- verify: `email` is the address being confirmed (differs from users.email for an email change).
CREATE TABLE email_tokens (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose text NOT NULL CHECK (purpose IN ('verify', 'reset')),
    email text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at timestamptz
);
CREATE INDEX email_tokens_user ON email_tokens (user_id, purpose);

-- Outbox: rendered at enqueue, sent by a worker with retries. Bodies are cleared once sent/failed
-- (they can carry single-use links). idem_key dedupes enqueues and is sent as Resend's Idempotency-Key.
CREATE TABLE email_outbox (
    id uuid PRIMARY KEY,
    idem_key text NOT NULL UNIQUE,
    to_email text NOT NULL,
    template text NOT NULL,
    subject text NOT NULL,
    html text,
    text_body text,
    status text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'sent', 'failed')),
    attempts int NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    last_error text,
    provider_id text,
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz
);
CREATE INDEX email_outbox_due ON email_outbox (next_attempt_at) WHERE status = 'queued';
-- Per-address rate limit counts recent rows.
CREATE INDEX email_outbox_to ON email_outbox (to_email, created_at DESC);
