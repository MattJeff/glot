-- GitHub App (« Connect GitHub ») : see crates/server/src/github_app.rs.
-- One app per glot deployment, created through the manifest flow; secrets encrypted with GLOT_SECRET_KEY.
CREATE TABLE github_app (
    id int PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    app_id bigint NOT NULL,
    slug text NOT NULL,
    client_id text NOT NULL,
    client_secret_enc bytea NOT NULL,
    private_key_enc bytea NOT NULL,
    webhook_secret_enc bytea NOT NULL,
    html_url text NOT NULL,
    created_by uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Single-use CSRF states (manifest creation, installation / OAuth callback), 1 h.
CREATE TABLE github_states (
    state text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id uuid REFERENCES projects(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Installations a glot user proved access to (GET /user/installations with their user token).
CREATE TABLE github_user_installations (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    installation_id bigint NOT NULL,
    account text NOT NULL,
    PRIMARY KEY (user_id, installation_id)
);

-- Project ↔ repository. pr_* = the single glot PR currently open (NULL when none).
CREATE TABLE github_links (
    project_id uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    installation_id bigint NOT NULL,
    repo text NOT NULL,
    branch text NOT NULL,
    pr_number int,
    pr_url text,
    pr_branch text,
    last_sha text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX github_links_repo ON github_links (lower(repo));
CREATE INDEX github_links_installation ON github_links (installation_id);
