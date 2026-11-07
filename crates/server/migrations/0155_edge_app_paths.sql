-- glot Edge application paths: pages that stay on the origin's own domain (login, account, payment, OAuth, API).
-- One rule per element: `/login` (app_action), `redirect /dashboard`, `proxy /api/` (see edge/app_paths.rs).
-- Existing rows keep '{}' (behavior unchanged); new rows get the suggested list.
ALTER TABLE edge_settings
    ADD COLUMN app_paths text[] NOT NULL DEFAULT '{}',
    ADD COLUMN app_action text NOT NULL DEFAULT 'redirect' CHECK (app_action IN ('redirect', 'proxy'));
ALTER TABLE edge_settings ALTER COLUMN app_paths
    SET DEFAULT '{/login,/register,/dashboard,/auth/,/oauth/,/settings,/account,/checkout,/billing,"proxy /api/"}';
