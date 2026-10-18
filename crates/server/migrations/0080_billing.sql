-- Billing (crates/server/src/billing.rs). One billing account per user: it covers the projects that user owns.
-- ponytail: no organizations yet; when they land, key billing_accounts/billing_usage by org instead of user.
CREATE TABLE billing_accounts (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    stripe_customer_id text UNIQUE,
    plan text NOT NULL DEFAULT 'free',                -- plan of the subscription (free | pro | team)
    status text NOT NULL DEFAULT 'none',              -- Stripe subscription status, 'none' without one
    stripe_subscription_id text,
    base_item_id text,                                -- subscription item of the plan price (plan changes swap it)
    metered boolean NOT NULL DEFAULT false,           -- subscription carries the usage-based LLM price
    current_period_end timestamptz,
    cancel_at_period_end boolean NOT NULL DEFAULT false,
    grace_until timestamptz,                          -- set by invoice.payment_failed, cleared by invoice.paid
    sub_event_at bigint,                              -- `created` of the last applied subscription event (out-of-order guard)
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Webhook idempotency: an event id is applied once (inserted in the same transaction as its effects).
CREATE TABLE stripe_events (
    id text PRIMARY KEY,
    type text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now()
);

-- Monthly counters per billing account (kind = Limits field: cdn_requests, insights_events, mcp_calls, ondemand_jobs, llm_strings).
CREATE TABLE billing_usage (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month date NOT NULL,
    kind text NOT NULL,
    n bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, month, kind)
);

CREATE INDEX project_members_owner ON project_members (user_id) WHERE role = 'owner';
