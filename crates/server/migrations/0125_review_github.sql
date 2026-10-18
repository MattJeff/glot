-- Live review → GitHub (review_live/github.rs): an annotation mirrored as an issue in the linked repo, or as a
-- comment on the open glot PR. A merged PR that closes the issue (`Fixes #12`) or is that PR resolves the annotation.
CREATE TABLE review_github (
    annotation_id uuid PRIMARY KEY REFERENCES review_annotations(id) ON DELETE CASCADE,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    repo text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('issue', 'pr_comment')),
    number integer NOT NULL,              -- issue number, or PR number for pr_comment
    url text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX review_github_repo ON review_github (lower(repo), number);
