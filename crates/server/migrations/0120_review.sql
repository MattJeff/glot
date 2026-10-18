-- Live review (docs/design/live-review-contract.md): annotations anchored on the customer's site, threads,
-- reactions, captures, shareable review links, history (also pushed live through NOTIFY glot_review).
CREATE TABLE review_settings (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    origins text[] NOT NULL DEFAULT '{}'   -- widget CORS allow-list ("https://app.example.com")
);

-- Widget tokens: shareable links (reviewers without an account) and 12 h member sessions. sha256 only.
CREATE TABLE review_links (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('link', 'member')),
    token_hash text NOT NULL UNIQUE,
    access text NOT NULL CHECK (access IN ('read', 'comment', 'edit', 'member')),
    label text,
    user_id uuid REFERENCES users(id) ON DELETE CASCADE,       -- member sessions
    created_by uuid REFERENCES users(id) ON DELETE SET NULL,
    expires_at timestamptz,
    revoked_at timestamptz,
    last_used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX review_links_project ON review_links (project_id, kind);

CREATE TABLE review_annotations (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'in_progress', 'resolved', 'wontfix')),
    anchor jsonb NOT NULL,
    url text NOT NULL,
    route text NOT NULL,
    locale text NOT NULL,
    key_id uuid REFERENCES keys(id) ON DELETE SET NULL,
    body text NOT NULL,
    suggestion text,
    author_user uuid REFERENCES users(id) ON DELETE SET NULL,
    author_link uuid REFERENCES review_links(id) ON DELETE SET NULL,
    author_name text NOT NULL,
    assignee uuid REFERENCES users(id) ON DELETE SET NULL,
    feedback_group_id uuid UNIQUE REFERENCES feedback_groups(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    resolved_at timestamptz
);
CREATE INDEX review_annotations_project ON review_annotations (project_id, updated_at DESC);
CREATE INDEX review_annotations_route ON review_annotations (project_id, route, locale);

CREATE TABLE review_comments (
    id uuid PRIMARY KEY,
    annotation_id uuid NOT NULL REFERENCES review_annotations(id) ON DELETE CASCADE,
    author_user uuid REFERENCES users(id) ON DELETE SET NULL,
    author_link uuid REFERENCES review_links(id) ON DELETE SET NULL,
    author_name text NOT NULL,
    body text NOT NULL,
    mentions uuid[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    edited_at timestamptz
);
CREATE INDEX review_comments_annotation ON review_comments (annotation_id, created_at);

-- actor = 'u:<user id>' or 'l:<link id>:<name>'; comment_id NULL = on the annotation itself.
CREATE TABLE review_reactions (
    annotation_id uuid NOT NULL REFERENCES review_annotations(id) ON DELETE CASCADE,
    comment_id uuid REFERENCES review_comments(id) ON DELETE CASCADE,
    actor text NOT NULL,
    emoji text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX review_reactions_unique
    ON review_reactions (annotation_id, coalesce(comment_id, '00000000-0000-0000-0000-000000000000'), actor, emoji);

CREATE TABLE review_attachments (
    id uuid PRIMARY KEY,
    annotation_id uuid NOT NULL REFERENCES review_annotations(id) ON DELETE CASCADE,
    content_type text NOT NULL,
    data bytea NOT NULL,
    width integer,
    height integer,
    author_name text NOT NULL,
    author_link uuid REFERENCES review_links(id) ON DELETE SET NULL,
    author_user uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX review_attachments_annotation ON review_attachments (annotation_id);

-- Append-only history; every row is NOTIFYed on glot_review (SSE).
CREATE TABLE review_events (
    id bigserial PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    annotation_id uuid,               -- no FK: history outlives a deleted annotation
    comment_id uuid,
    kind text NOT NULL,
    actor_user uuid,
    actor_link uuid,
    actor_name text NOT NULL,
    data jsonb NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX review_events_project ON review_events (project_id, id);
CREATE INDEX review_events_annotation ON review_events (annotation_id, id);

-- Shared per-minute budget for widget writes, same scheme as feedback_rate.
CREATE TABLE review_rate (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    window_start timestamptz NOT NULL,
    hits integer NOT NULL
);
