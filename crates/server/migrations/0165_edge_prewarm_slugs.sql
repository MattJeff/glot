-- glot Edge, international SEO:
--   prewarm           = glot crawls the origin's sitemap (rate-limited, robots.txt respected) so pages are translated
--                       before a visitor or a crawler asks for them; `prewarm_pages` = pages fetched per run.
--                       New segments still go through the daily cap / monthly budget of the project.
--   translated_slugs  = opt-in: `/pricing` is served as `/precios` on the Spanish site (edge_slugs), the source
--                       path redirects to it, canonical / hreflang / sitemaps use it.
ALTER TABLE edge_settings
  ADD COLUMN prewarm boolean NOT NULL DEFAULT true,
  ADD COLUMN prewarm_pages integer NOT NULL DEFAULT 200,
  ADD COLUMN translated_slugs boolean NOT NULL DEFAULT false;

-- One row per project: the last / running pre-warm.
CREATE TABLE edge_prewarm (
  project_id  uuid PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  status      text NOT NULL DEFAULT 'running',   -- running | done | failed
  pages_total integer NOT NULL DEFAULT 0,
  pages_done  integer NOT NULL DEFAULT 0,
  skipped     integer NOT NULL DEFAULT 0,        -- robots.txt, application paths, non-HTML, errors
  segments    integer NOT NULL DEFAULT 0,        -- new segments accepted
  error       text,
  started_at  timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz
);

-- Translated URL paths. A slug never changes by itself: once written it is stable; replacing it keeps the old
-- row (`current = false`) so the old URL redirects to the new one.
CREATE TABLE edge_slugs (
  project_id  uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  locale      text NOT NULL,
  slug        text NOT NULL,                     -- localized path, `/precios`
  source_path text NOT NULL,                     -- `/pricing`
  current     boolean NOT NULL DEFAULT true,
  origin      text NOT NULL DEFAULT 'machine',   -- machine | human
  created_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, locale, slug)
);
CREATE UNIQUE INDEX edge_slugs_current ON edge_slugs (project_id, locale, source_path) WHERE current;
