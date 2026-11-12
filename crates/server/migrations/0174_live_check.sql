-- "Is this language really live on my site?" (crates/server/src/live_check.rs, locale_alias.rs).
-- Code the site uses for a glot locale when it differs: {"zh-CN": "zh"}. An entry equal to its key means
-- "keep glot's code" (the automatic alias is not re-applied).
ALTER TABLE projects ADD COLUMN site_codes jsonb NOT NULL DEFAULT '{}'::jsonb;

-- What the switcher saw on real pages after a visitor picked a language: counters per (day, locale).
-- No IP, user agent, URL path, cookie or visitor id: the last <html lang> and page origin only.
CREATE TABLE live_check_beacons (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    day date NOT NULL,
    applied bigint NOT NULL DEFAULT 0,  -- the page showed the language
    ignored bigint NOT NULL DEFAULT 0,  -- the page stayed as it was
    html_lang text NOT NULL DEFAULT '', -- <html lang> of the last ignored page
    origin text NOT NULL DEFAULT '',
    PRIMARY KEY (project_id, locale, day)
);

-- Last server-side probe of the site's home page per locale (live_check::Probe as JSON).
CREATE TABLE live_check_probes (
    project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    locale text NOT NULL,
    result jsonb NOT NULL,
    checked_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, locale)
);
