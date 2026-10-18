-- Back-translation review cache (glot_llm::review), valid while source/target hashes match.
CREATE TABLE translation_reviews (
    key_id uuid NOT NULL REFERENCES keys(id) ON DELETE CASCADE,
    locale text NOT NULL,
    source_hash text NOT NULL,
    target_hash text NOT NULL,
    back_translation text NOT NULL,
    verdict text NOT NULL CHECK (verdict IN ('same', 'nuance', 'different')),
    explanation text NOT NULL,
    suggestion text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (key_id, locale)
);
