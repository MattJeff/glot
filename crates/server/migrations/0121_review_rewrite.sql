-- Live review RV4: AI rewrite proposals, cached per (key, instruction, context) hash (glot_llm::rewrite::RewriteRequest::cache_key).
CREATE TABLE review_rewrites (
    id uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    cache_key text NOT NULL,
    result jsonb NOT NULL,           -- glot_llm::rewrite::Rewrite
    cost_usd double precision NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (project_id, cache_key)
);
