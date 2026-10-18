-- Onboarding checklist state + milestones, one row per project (onboarding.rs).
CREATE TABLE onboarding (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    path text CHECK (path IN ('agent', 'terminal', 'github')),
    hidden boolean NOT NULL DEFAULT false,
    token_created_at timestamptz,
    badge_seen_at timestamptz
);

-- Milestones are never revoked once reached.
CREATE TABLE achievements (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    id text NOT NULL,
    unlocked_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, id)
);
