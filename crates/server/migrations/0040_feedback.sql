-- Native-speaker feedback from production (@glot/feedback): reports grouped by (key, locale), AI proposal, optional GitHub PR.
-- Privacy: no IP, user agent, cookie or visitor id. url is stored without query/fragment, dom is a tag path (no text).
CREATE TABLE feedback_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    -- reports on one (key, locale) before the server asks the LLM for a correction
    threshold integer NOT NULL DEFAULT 3 CHECK (threshold > 0),
    github_repo text,                 -- owner/name
    github_base text NOT NULL DEFAULT 'main',
    catalog_path text NOT NULL DEFAULT 'messages/{locale}.json',
    lock_path text NOT NULL DEFAULT 'glot.lock',
    github_token_enc bytea            -- AES-GCM (GLOT_SECRET_KEY), write-only like the LLM key
);

CREATE TABLE feedback_groups (
    id uuid PRIMARY KEY,
    key_id uuid NOT NULL REFERENCES keys(id) ON DELETE CASCADE,
    locale text NOT NULL,
    status text NOT NULL DEFAULT 'open', -- open | proposed | accepted | rejected
    proposal text,
    previous text,                    -- translation the proposal replaces
    back_translation text,
    verdict text,
    explanation text,
    issues jsonb NOT NULL DEFAULT '[]',
    pr_url text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
-- At most one live group per (key, locale); decided groups are kept as history.
CREATE UNIQUE INDEX feedback_groups_live ON feedback_groups (key_id, locale) WHERE status IN ('open', 'proposed');

CREATE TABLE feedback_reports (
    id bigserial PRIMARY KEY,
    group_id uuid NOT NULL REFERENCES feedback_groups(id) ON DELETE CASCADE,
    kind text NOT NULL,               -- wrong | awkward | unclear
    text text NOT NULL,
    suggestion text,
    url text,
    dom text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX feedback_reports_group ON feedback_reports (group_id);

-- Shared per-minute budget for POST /api/feedback/{key}, same scheme as insight_rate.
CREATE TABLE feedback_rate (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    window_start timestamptz NOT NULL,
    hits integer NOT NULL
);
