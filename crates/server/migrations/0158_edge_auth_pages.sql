-- Pages behind sign-in (dashboards, accounts) — how glot Edge treats a request that carries a session:
--   server  = translated by the Edge per response from the TM (Cache-Control: private, never shared-cached,
--             never discovered) — the default, same behaviour as before this column;
--   patcher = the Edge serves the origin untouched; the site's own `<script data-glot-app>` tag translates in
--             the browser (same domain: session cookies untouched);
--   off     = the origin untouched, untranslated.
-- auth_discover: the app-mode patcher may report UI text (whitelisted selectors only; every new segment waits
-- for a review before being served). Off by default.
ALTER TABLE edge_settings
  ADD COLUMN auth_pages text NOT NULL DEFAULT 'server' CHECK (auth_pages IN ('off', 'patcher', 'server')),
  ADD COLUMN auth_discover boolean NOT NULL DEFAULT false;
