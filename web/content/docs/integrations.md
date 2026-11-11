# Integrations

`glot init` already writes per-project rules for agents (`CLAUDE.md`, `AGENTS.md`, `.claude/skills/glot`, `.cursor/rules/glot.mdc`, `.mcp.json`). The integrations below are for **before** a project uses glot, so the agent reaches for it when someone asks for a multilingual app. Source files: [`integrations/`](https://github.com/MattJeff/glot/tree/main/integrations).

## Claude Code

```sh
claude plugin marketplace add MattJeff/glot
claude plugin install glot@glot
```

The plugin adds:

- a `glot` skill, which Claude uses whenever you ask for a multilingual app or new UI text;
- `/glot:i18nize [paths]`: convert hardcoded strings, then translate;
- `/glot:translate [locales]`: translate the delta and check;
- the MCP server (pinned `glot-i18n` version).

MCP only, without the plugin: `claude mcp add glot -- npx -y glot-i18n mcp`. See [MCP server](mcp.md) for the tools.

## Cursor

Copy [`agents/cursor/glot.mdc`](https://github.com/MattJeff/glot/blob/main/integrations/agents/cursor/glot.mdc) to `.cursor/rules/`, and add the MCP server to `.cursor/mcp.json`:

```json
{ "mcpServers": { "glot": { "command": "npx", "args": ["-y", "glot-i18n", "mcp"] } } }
```

Windsurf: copy [`agents/windsurf/glot.md`](https://github.com/MattJeff/glot/blob/main/integrations/agents/windsurf/glot.md) to `.windsurf/rules/`; same MCP JSON in `~/.codeium/windsurf/mcp_config.json`.

## Codex

Append [`agents/AGENTS.md`](https://github.com/MattJeff/glot/blob/main/integrations/agents/AGENTS.md) to your `AGENTS.md` (also read by other agents that follow the convention), then:

```sh
codex mcp add glot -- npx -y glot-i18n mcp
```

## Lovable, Bolt, v0, Replit

AI app builders generate English-only apps with hardcoded strings. Two steps:

1. **Before generating**: paste the [knowledge snippet](https://github.com/MattJeff/glot/blob/main/integrations/ai-builders-prompt.md) into the project instructions (Lovable: Project settings → Knowledge; Bolt: project prompt; v0: project instructions; Replit: `replit.md`). Every string then goes through next-intl or react-i18next from the first prompt.
2. **After generating**: push to GitHub and run the **Translate this app** workflow ([`glot-onboard.yml`](https://github.com/MattJeff/glot/blob/main/integrations/workflows/glot-onboard.yml)). One run installs the i18n library if missing, runs `glot init`, `glot i18nize` and `glot translate` (if `ANTHROPIC_API_KEY` is set as a secret) and opens a pull request with coverage and the `glot check` result.

Lovable commits straight to `main`, so PR-triggered workflows do not see its edits: re-run the onboard workflow to catch up. Replit has a shell, so run the CLI directly:

```sh
npx -y glot-i18n init --yes --targets fr,es,de
npx -y glot-i18n i18nize --yes
npx -y glot-i18n translate --yes
```

Full per-tool guide: [`ai-builders.md`](https://github.com/MattJeff/glot/blob/main/integrations/ai-builders.md).

## GitHub Action

```yaml
on: pull_request
permissions: { contents: write, pull-requests: write }
jobs:
  i18n:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: ${{ github.event.pull_request.head.repo.full_name }}
          ref: ${{ github.head_ref }}
      - uses: MattJeff/glot/action@v0
        with:
          # Fork PRs get no secrets: only check them, translate on same-repo branches.
          command: ${{ github.event.pull_request.head.repo.full_name == github.repository && 'both' || 'check' }}
          anthropic-api-key: ${{ secrets.ANTHROPIC_API_KEY }}
```

It validates catalogs (inline annotations), translates the delta, commits the catalogs, `glot.lock` and `.glot/layout.json` to the PR branch (nothing else), and keeps one PR comment up to date with coverage, issues and cost. The binary is pinned to the action release and verified against the release's `SHA256SUMS`.

| Input | Default | |
|---|---|---|
| `command` | `check` | `check`, `translate` or `both` |
| `working-directory` | `.` | Directory containing `glot.toml` |
| `version` | pinned to the action release | glot release tag, or `latest`; skipped when `glot` is on PATH |
| `anthropic-api-key` | | Exported as `ANTHROPIC_API_KEY` |
| `openai-api-key` | | Exported as `OPENAI_API_KEY` |
| `llm-api-key` | | Exported under `llm.api_key_env` (else the provider's default variable) |
| `anthropic-workspace-id` | | Exported as `ANTHROPIC_WORKSPACE_ID` (org-level keys `sk-ant-usr-…`) |
| `federation-rule-id`, `organization-id`, `service-account-id` | | Keyless Claude via Workload Identity Federation (needs `permissions: id-token: write`), see [below](github.md#keyless-claude-access-with-workload-identity-federation) |
| `workspace-id` | | `wrkspc_…`, when the federation rule spans several workspaces |
| `audience` | `https://api.anthropic.com` | Audience of the GitHub OIDC token; must match the rule |
| `comment` | `true` | Post or update the summary comment |
| `fail-on-error` | `true` | Fail the job when `glot check` reports errors |
| `github-token` | `github.token` | Used to comment |
| `token` | | PAT or GitHub App token used to push. Commits pushed with the default `GITHUB_TOKEN` do not trigger CI runs; set this if required checks must run on the translation commit |

Permissions: `contents: write` to push (`contents: read` is enough for `command: check`), `pull-requests: write` to comment. Details in [`action/README.md`](https://github.com/MattJeff/glot/blob/main/action/README.md).

Output: `check-exit-code`.

## In-app overlay

Edit translations in your running app. Start the local server in the project, then load the overlay in development:

```sh
glot dev                 # binds 127.0.0.1:4317, prints a token
```

```tsx
// app/layout.tsx (Next.js App Router)
import { GlotOverlay } from "@glot-i18n/overlay/react";
{process.env.NODE_ENV === "development" && <GlotOverlay />}
```

Or without installing anything: `<script src="http://127.0.0.1:4317/overlay.js" defer></script>`.

**Alt+click** any text: a popover shows its key, its value in every locale (editable, validated on save), open issues, whether it overflows, and AI actions (Suggest, Shorten, More formal, Translate all). Text without a key can be extracted to a new key. Saves go to your catalog files, so HMR reloads the page. **Alt+Shift+G** toggles inspect mode. The overlay is ~6 kB gzipped and never starts in production.

## Coverage badge

```sh
integrations/badge/glot-badge.sh --push   # commits glot-badge.json to an orphan `glot-badge` branch
```

```md
![glot](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/OWNER/REPO/glot-badge/glot-badge.json)
```

Renders as "translated by glot · N languages · X%".

## CMS connectors: how sync works

Dashboard → **Integrations** connects a CMS or a store to a project. Every connector follows the same loop:

1. **Pull**: glot reads the source-language content page by page (cursor or page numbers). If a sync is interrupted (restart, CMS error), the next one resumes from the last saved page.
2. **Keys**: each translatable field becomes a glot key `{cms}:{resource}:{id}:{field}` (`shopify:product:42:title`). The content type and title go into the key description, so the translator gets context. HTML and rich text are never sent whole: they are split into text segments (inline tags become `<0>…</0>`, rich text marks stay on their nodes), one key `{key}#{hash}` per segment. Markup, attributes, Gutenberg block comments and rich text structure are rebuilt byte for byte.
3. **Translate**: a `connector_translate` job (Jobs page: *CMS translation*) translates missing and stale keys with the project's model, validators and glossary. It sends no email.
4. **Push**: a field is written back only when **every** segment has a valid translation (`machine` or `reviewed`, never `stale`), so a page is never half-translated. glot stores a hash of each value pushed per locale. Running a sync again with nothing changed writes nothing, and a source change re-sends only the fields that changed.
5. **Triggers**: **Sync now** on the card, a webhook from the CMS, and a periodic push every 10 minutes that picks up translations reviewed in glot since the last push. The periodic push compares hashes first, so it makes no CMS calls when nothing changed.

Locales: each glot target locale is written to the CMS locale with the same code. Map them differently with `PATCH /api/projects/{slug}/connectors/{id}` and `{"settings": {"locale_map": {"pt-BR": "pt-br"}}}`.

Webhooks: every card shows a webhook URL that carries its own token. Paste it into CMSs that cannot sign their calls. CMSs that do sign (Shopify) are verified with their own secret. A webhook queues an incremental sync. If a sync is already running, another one starts when it finishes.

Security: credentials are encrypted at rest (AES-256-GCM, server key `GLOT_SECRET_KEY`). They are never logged and never sent back to the browser: the dashboard only lists which fields are set. CMS URLs must resolve to public addresses (no internal network), with no redirects. glot waits and retries on `429`/`503`, following `Retry-After`.

API: `GET/POST /api/projects/{slug}/connectors`, `PATCH/DELETE …/connectors/{id}`, `POST …/{id}/test` (credentials, CMS locales), `POST …/{id}/sync` (`{"incremental": true}` for changes since the last sync only), `GET …/{id}/status` (job progress, fields pushed per locale).

Limits: deleting content in the CMS removes it from sync on the next full sync, but its glot keys stay. glot does not translate URL slugs, images or references.

## Shopify

Dashboard → **Integrations** → Shopify. glot uses the GraphQL Admin API ([`translatableResources`](https://shopify.dev/docs/api/admin-graphql/latest/queries/translatableResources), [`translationsRegister`](https://shopify.dev/docs/api/admin-graphql/latest/mutations/translationsRegister), [guide](https://shopify.dev/docs/apps/build/markets/manage-translated-content)), API version `2025-10` (`settings.api_version` overrides it).

- **Credentials**: in Shopify admin → Settings → Apps and sales channels → Develop apps, create a custom app. Give it the Admin API scopes `read_translations`, `write_translations`, `read_products`, `read_content`, `read_online_store_pages`, `read_themes` and `read_locales`/`write_locales`, install it, then copy the **Admin API access token** (`shpat_…`) and the store domain (`my-store.myshopify.com`). The **API secret key** is optional: with it, glot registers webhooks and verifies them.
- **What is translated**: products, product options and values, collections, pages, blogs, articles, metafields, metaobjects, menus and links, shop, policies and the online store theme (`ONLINE_STORE_THEME`). `settings.resources` (`["PRODUCT", "COLLECTION"]`) limits the list. HTML fields (`body_html`, `HTML`, `INLINE_RICH_TEXT`) keep their tags. Rich text metafields (JSON) keep their nodes. URLs, file references, JSON values and handles (URL slugs) are skipped. Set `settings.translate_handles: true` to translate handles.
- **Digests**: each translation is registered with the `translatableContentDigest` of the source it was made from. If the merchant edits the source in between, Shopify refuses the translation. The next sync then re-translates the new source.
- **Locales**: a glot target locale missing from the store is added with `shopLocaleEnable`, unpublished. You publish it in Shopify (Settings → Languages) when you're ready.
- **Rate limits**: glot follows the [calculated query cost](https://shopify.dev/docs/api/usage/limits). Before the next call, it waits until the bucket has refilled enough for it, and it retries `THROTTLED` responses.
- **Webhooks**: with the API secret key, glot subscribes `PRODUCTS_CREATE`, `PRODUCTS_UPDATE`, `COLLECTIONS_CREATE` and `COLLECTIONS_UPDATE` to the card's webhook URL, and checks `X-Shopify-Hmac-Sha256` (base64 HMAC-SHA256 of the body). Pages, articles and theme edits are picked up by **Sync now** or the next webhook.
- **Theme files**: if your theme lives in Git, glot can already translate `locales/*.json` there (CLI, GitHub). The Shopify connector covers what isn't in theme files: store content and the theme as edited in the admin. Use one path for theme strings, not both: drop `ONLINE_STORE_THEME` from `settings.resources` when the theme's locale files are handled in Git.

## Shopify app embed

A theme app extension that puts the glot script and a language switcher on every page of your storefront, without editing theme code. Source: [`integrations/shopify-app`](https://github.com/MattJeff/glot/tree/main/integrations/shopify-app). It works with Online Store 2.0 themes, the only themes where [theme app extensions](https://shopify.dev/docs/apps/build/online-store/theme-app-extensions) run. Vintage themes need the script tag pasted into `theme.liquid` instead.

It's not on the Shopify App Store. You install it as your own custom app:

1. In the [Dev Dashboard](https://dev.shopify.com/dashboard/), create an app (**Apps → Create app → Start from Dev Dashboard**, [guide](https://shopify.dev/docs/apps/build/dev-dashboard/create-apps-using-dev-dashboard)).
2. With [Shopify CLI](https://shopify.dev/docs/apps/build/cli-for-apps), from `integrations/shopify-app`: `shopify app config link` (writes your `client_id` into `shopify.app.toml`), then `shopify app deploy` (releases a version with the extension).
3. In the Dev Dashboard, **Install app** on your store.
4. In the theme editor: **App embeds** → turn on **glot**, then fill in your project slug and public key (`pk_…`, from **glot Edge → Website tag**), and pick a switcher and its corner. App embeds are off after install until you turn them on.
5. In glot, turn the [website tag](/docs/website) **on** (**glot Edge → Website tag**) and pick the languages there.

The files follow Shopify's reference: [`shopify.app.toml`](https://shopify.dev/docs/apps/build/cli-for-apps/app-configuration) (`name`, `client_id`, `application_url`, `embedded`, `[access_scopes]`, `[auth]`, `[webhooks]`), [`shopify.extension.toml`](https://shopify.dev/docs/apps/build/online-store/theme-app-extensions/configuration) (`type = "theme"`) and an app embed block with `"target": "body"`. Shopify injects that block before `</body>`. The embed needs no Admin API scope.

Switcher modes:

- **Store languages** (default): Shopify's own [localization form](https://shopify.dev/docs/storefronts/themes/markets/multiple-currencies-languages) (`language_code`), so each language keeps its Shopify URL (`/es/…`) and Shopify's hreflang. glot.js follows the store's language instead of choosing its own: the embed saves `request.locale` as the visitor's choice before the tag runs, and hides glot's menu. Use this with the **Shopify** connector above: the connector writes products, collections, pages and the theme into Shopify through the Translations API (that's what search engines index), and glot.js translates the rest in the browser (text added by other apps, for example).
- **glot menu**: the website tag's menu in the chosen corner (`data-position`), same URL for every language. Nothing is indexed per language unless you add glot Edge. Fits stores without Shopify languages.
- **None**: your own switcher calling `glot.setLocale("es")`.

Customer data: the embed is off on customer account templates (`customers/*`) and the password page, and Shopify never renders app embeds on checkout. Form values are never read by glot.js.

Limits: the connector still needs an Admin API access token (`shpat_…`) from a custom app created in the store admin. Since January 1, 2026, new custom apps are created in the Dev Dashboard ([Shopify Help](https://help.shopify.com/en/manual/apps/app-types/custom-apps)), and those get their tokens through the [client credentials grant](https://shopify.dev/docs/apps/build/authentication-authorization/client-credentials-grant), valid 24 hours. glot's connector doesn't renew these tokens yet. Stores with a legacy custom app keep using its token. `pnpm test` renders the embed with liquidjs, with Shopify's own tags stubbed. It hasn't been deployed to a real store.

## WordPress

Dashboard → **Integrations** → WordPress. glot uses the [REST API](https://developer.wordpress.org/rest-api/reference/posts/) with an [application password](https://developer.wordpress.org/rest-api/using-the-rest-api/authentication/#basic-authentication-with-application-passwords) (Users → Profile → Application passwords). Use an Editor account. Never give glot your login password.

- **What is translated**: posts and pages (title, content, excerpt, every status except trash), categories and tags (name, description). Menu items (title) are opt-in with `settings.menus: true`. Content is read raw (`context=edit`). Gutenberg HTML keeps its block comments (`<!-- wp:paragraph {…} -->`), attributes and markup: only the text between tags is translated.
- **Multilingual plugin**: WordPress has no translations of its own, so glot writes into the site's plugin. It detects the plugin from the REST index (`settings.plugin: "polylang" | "wpml"` forces one).
  - **Polylang Pro**. The REST API is a [Pro feature](https://polylang.pro/doc/rest-api/). glot reads languages from `/pll/v1/languages` and pulls only the default language (`?lang=en`). It updates the linked translation in place, or creates it with `?lang=fr&translations[en]=12` so it's linked to its source.
  - **WPML**: glot pulls with `?lang=` and finds existing translations in `wpml_translations`. It creates a missing one with `?lang=fr&translation_of=12`. WPML documents this parameter for its WooCommerce endpoints, not for core posts. If your WPML version ignores it, create the translation once in WPML, and glot keeps it updated from then on. WPML has no languages endpoint: set `settings.languages` and `settings.source_lang`.
  - **No plugin**, or Polylang free (no REST API): glot can read and translate, but has nowhere to write. Pushes fail with a message saying so.
- **New translations** are created as `draft` so you review them in WordPress. Set `settings.new_status` to `publish`, or to `same` to copy the source's status.
- **Webhooks**: WordPress core sends none. Make a plugin (WP Webhooks, or a `save_post` hook) call the card's webhook URL. You can also sign the body with `X-Glot-Signature: sha256=<hex HMAC-SHA256>`, using the token as key. Posts and pages are then re-read with `modified_after`.
- **Limits**: page builders that store text outside `post_content` (Elementor, Divi data in post meta), custom fields (ACF), and WooCommerce products aren't synced. Menus aren't linked by Polylang's REST API: create the menu per language in the plugin first.

## WordPress plugin

A WordPress plugin, installed from a zip. It isn't on WordPress.org yet. Source: [`integrations/wordpress-plugin`](https://github.com/MattJeff/glot/tree/main/integrations/wordpress-plugin). It needs WordPress 6.1+ and PHP 7.4+. Upload `glot.zip` (Plugins → Add New → Upload Plugin), activate it, then open **Settings → glot**.

- **Settings**: project slug, public key (`pk_…`, from **glot Edge → Website tag**; a secret `glot_…` key is refused, since this one is printed in your pages), switcher position, SEO mode. The server and script URLs only change for a self-hosted glot. Languages, and turning the tag on, are set in glot (**glot Edge → Website tag**), like for any site.
- **Script tag**: adds the [website tag](/docs/website) (`https://app.glotlang.com/widget/glot.js`) to every front-end page, with `data-project`, `data-key` and `data-server`. It's printed in the footer, so the plugin already knows whether the page placed a switcher. The tag is `async` and waits for the page anyway.
- **Switcher**: the `[glot_switcher]` shortcode, a **glot language switcher** widget, a block of the same name, and an Elementor widget (same name, **General** category). Each one outputs a `[data-glot-switcher]` placeholder, and the menu goes there, inline (`data-position="[data-glot-switcher]"`). The tag fills the first one, so use one per page. On other pages, the menu goes where you picked: as set in glot, a floating corner, or hidden (`data-switcher="none"`, your own controls call `glot.setLocale("es")`).
- **WooCommerce**: glot.js never reads or sends form values, so checkout, cart and account forms are safe. Customer details and addresses (order details, My account → Addresses) are wrapped in `data-glot-user`, so they're never translated or reported. Cart, checkout and account pages get no hreflang. With glot Edge, these pages carry WooCommerce's session cookie and follow your [Pages behind sign-in](/docs/edge#pages-behind-sign-in) setting.
- **SEO mode**: glot Edge serves the translated pages on their own URLs: a subdomain (`es.example.com`, CNAME) or a subdirectory (`example.com/es/…` through the Cloudflare Worker or a proxy rule on your host). Set the domain up in **glot Edge → Setup** first. The plugin then puts the hreflang links (each ready language, the source, `x-default`) in the HTML of every WordPress page, where crawlers read them. The website tag only adds them in the browser. The Edge URLs come from `hreflang` in `/edge/v1/{project}/status`: verified domains and ready languages only, cached 1 hour. If glot can't be reached, the plugin keeps the last list. This is the plugin's only server-side request on the front end, and only in SEO mode.

Why the plugin doesn't serve `/es/…` from WordPress itself: a rewrite rule (`add_rewrite_rule`) works, but each translated page view would then wait on a PHP request to glot, page cache plugins would store a second copy of every page, and the site would slow down whenever glot did. glot Edge already serves translated HTML (first byte translated, canonical, hreflang, sitemaps) in front of WordPress. The plugin points search engines to it.

**Connect to glot** (optional) links the [WordPress connector](#wordpress), so translations are also written into Polylang Pro or WPML posts:

1. Save your project slug, paste a glot API token (`glot_…`, dashboard → Settings → API tokens), and click **Connect to glot**.
2. The plugin creates an application password named `glot` for your account and calls `POST /api/projects/{slug}/connectors` with your site URL, username and that password. glot tests them against your site before accepting. The token is used once and never stored.
3. From then on, saving a post, page, category or tag calls the connector's webhook URL, so glot re-syncs right away. WordPress core sends no webhooks of its own.

Your site must be reachable from the internet, with pretty permalinks (`/wp-json/`), and application passwords must be enabled (HTTPS, and no security plugin turning them off). **Disconnect** revokes the application password. Delete the WordPress card in glot as well. Uninstalling removes the plugin's options and revokes the password.

Security: settings and connection need `manage_options`, the forms carry nonces, all output is escaped, and errors are kept in a transient, never put in the URL. The plugin speaks English and French (`languages/`).

Limits: PHPUnit covers the pure logic (settings validation, hreflang URLs, script tag, connector answer). Front-end hooks were checked with WordPress functions stubbed. The plugin hasn't run on a live WordPress, Elementor or WooCommerce site yet. Customer data printed outside the two wrapped WooCommerce templates (an order note, a theme's own templates) isn't marked: add `data-glot-user` there yourself.

## Webflow

Dashboard → **Integrations** → Webflow. glot uses the Data API v2 with [Localization](https://developers.webflow.com/data/docs/working-with-localization): translations go into the site's **secondary locales**, so Localization must be enabled and the locales added in Webflow first. Each glot target locale is matched to a Webflow locale tag (`fr` ↔ `fr-FR`). If no Webflow locale matches, the push for that locale fails with a message naming it.

- **Token**: a site API token (Site settings → Apps & integrations → API access) with read and write access to CMS, Pages, Components and Sites. **Site ID** is only needed when the token can reach several sites.
- **What is translated**:
  - CMS items: `PlainText` and `RichText` fields of every collection. Rich text is HTML: glot translates the text segments and keeps every tag. Slugs, references, images and options are not touched. Archived items are skipped. Translations are written per locale with `PATCH /collections/{id}/items` and `cmsLocaleId`.
  - Static page content and components: text nodes, component property overrides, form placeholders and button labels, read with `GET /pages/{id}/dom` and written with `POST /pages/{id}/dom?localeId=…` (components: `/sites/{site}/components/{id}/dom`). Webflow does not let the API change primary-locale content, and glot never tries. Workspace-library components (`readonly`) are skipped.
- **Publishing**: written CMS items are staged. Set the `publish` setting to publish them in their locale after each push: `PATCH /api/projects/{slug}/connectors/{id}` with `{"settings": {"publish": true}}`. Static page and component content goes live with your next site publish.
- **Webhooks**: in Webflow, Site settings → Apps & integrations → Webhooks, add the card's webhook URL (it carries its own token) for `collection_item_created`, `collection_item_changed` and `site_publish`. Each call starts an incremental sync, which only reads items and pages changed since the last sync. If your webhooks are signed by Webflow (OAuth app), put the signing secret in **Webhook signing secret**. glot then checks `x-webflow-signature` (HMAC-SHA256 of `timestamp:body`) and rejects calls older than 5 minutes.
- **Limits**: Webflow allows 60 requests per minute on most plans. glot waits and retries on `429`, so a first sync of a large site takes a few minutes. Select choices and image alt text are not synced.

## Framer

Framer has two official ways in, and glot ships both in [`integrations/framer-plugin`](https://github.com/MattJeff/glot/tree/main/integrations/framer-plugin). Both read the project's [localization sources](https://www.framer.com/developers/plugins-localization) (pages, CMS items, components, settings), send the text to glot, and write back the translations with `setLocalizationData`.

- **Plugin** (in the editor): one *Sync with glot* button in Framer's Localization panel.
- **Server API script** (no editor open, for cron or CI): uses Framer's [Server API](https://www.framer.com/developers/server-api-introduction) through the official `framer-api` package (Node 22+).

Framer's Server API is a WebSocket protocol, documented only through its JavaScript SDK. That is why the dashboard has no Framer card: the sync runs where the SDK runs, which is the plugin or your script, never the glot server.

```sh
cd integrations/framer-plugin && pnpm install
# Plugin: run the dev server, then in Framer open Plugins → Development → Open Development Plugin
pnpm dev
# Or build plugin.zip to submit or share: pnpm build && pnpm run pack
# Server API: API key from the Framer project's Site settings → API Keys
FRAMER_API_KEY=… GLOT_TOKEN=glot_… pnpm sync https://framer.com/projects/Site--abc123 my-project [--publish]
```

- **Credentials**: a glot API token (dashboard → Settings → API tokens) and the project slug. The plugin stores the token in your browser only (never in the Framer project) and calls `/api/plugin/…`. Those routes accept any origin, but only with a bearer token. A session cookie is never accepted there.
- **Keys**: `framer:{group type}:{group id}:{source id}`, with the page or item name as translator context. Framer source types `string`, `formattedText` (HTML, tags kept) and `altText` are translated. `slug` and `link` are left to Framer.
- **What gets written**: a translation is written only when glot has it for the current source text (machine or reviewed, never stale). Machine translations are flagged *Needs review* in Framer. A translation reviewed in glot is marked done. A value someone marked done in Framer is kept unless you tick **Overwrite values marked done** (`--force`). Values Framer locks (managed collections) are never touched. Running the sync again with nothing changed writes nothing.
- **Limits**: nothing syncs automatically while the plugin is closed. Use the Server API script on a schedule for that. A Framer locale with no matching glot target locale is listed and skipped. `--publish` publishes the project and deploys it to production after a run that wrote something.

## Contentful

Dashboard → **Integrations** → Contentful. glot uses the [Content Management API](https://www.contentful.com/developers/docs/references/content-management-api/) (`https://api.contentful.com`; set **API URL** to `https://api.eu.contentful.com` for EU data residency).

- **Credentials**: the **Space ID** (Settings → General settings), a **personal access token** (`CFPAT-…`, Settings → CMA tokens) from a user who can edit entries, and the **environment** (`master` if empty).
- **What is translated**: entries of every content type, the fields with *Enable localization of this field* switched on: Short text (`Symbol`), Long text (`Text`) and Rich Text. Values are read in the space's default locale, which glot compares with the project's source language. `settings.content_types` (`["blogPost", "page"]`) limits the content types. Non-localized fields, references, media, slugs (as non-localized Short text) and JSON fields are left alone.
- **Structure kept**: in Rich Text, only the `text` nodes are translated: marks, hyperlinks, embedded entries and assets, lists and tables keep their place. Text with the `code` mark is not translated. Long text is treated as Markdown: each line, list item or table cell is translated on its own, and the syntax (`##`, `-`, `>`, `1.`, `|`) and code blocks stay exactly as written.
- **Push**: glot reads the entry, sets the locale's values and writes it with `PUT /entries/{id}` and `X-Contentful-Version`. If someone edited the entry in between (`409`), glot reads it again and retries once. Entries are left as drafts until you publish them. With `{"settings": {"publish": true}}`, glot publishes an entry after its update only if it was published and up to date before, so it never publishes someone's unfinished draft.
- **Locales**: the glot target locale must exist in the environment (Settings → Locales) under the same code, or be mapped with `settings.locale_map` (`{"fr": "fr-FR"}`).
- **Webhooks**: Settings → Webhooks → Add webhook, paste the card's webhook URL (it carries its token), triggers *Entry: save, auto save, publish*. Instead of the token in the URL, you can add a secret header `X-Glot-Webhook-Secret` with the token's value. Each call starts an incremental sync (`sys.updatedAt[gte]`).
- **Limits**: the CMA allows about 7 requests per second; glot waits on `429`. Arrays of short text (tags), JSON objects and asset titles are not synced.

## Sanity

Dashboard → **Integrations** → Sanity. glot uses the [HTTP API](https://www.sanity.io/docs/http-api): [GROQ queries](https://www.sanity.io/docs/http-query) (`perspective=raw`) and [mutations](https://www.sanity.io/docs/http-mutations), API version `v2025-02-19` (`settings.api_version` overrides it).

- **Credentials**: the **Project ID** and **dataset** (sanity.io/manage), and an API token with **Editor** permissions (manage → API → Tokens). **Source language code** is your schema's language key, `en` by default.
- **Two i18n models**, detected automatically (a dataset with `translation.metadata` documents is document-level), or forced with `settings.mode: "field" | "document"`:
  - **Field-level** ([`@sanity/language-filter`](https://www.sanity.io/plugins/language-filter) objects such as `{_type: "localeString", en: "…", fr: "…"}`, and [`sanity-plugin-internationalized-array`](https://www.sanity.io/plugins/internationalized-array) items `{_key: "fr", value: "…"}`): glot reads the source key of each localized object or array, at any depth. It writes `title.fr` with a `set` patch, or replaces the `fr` item of an internationalized array.
  - **Document-level** ([`@sanity/document-internationalization`](https://www.sanity.io/plugins/document-internationalization)): glot reads the documents whose `language` is the source language (`settings.language_field` renames the field). It translates their text strings and Portable Text. If the linked translation exists in `translation.metadata`, glot patches it. If it doesn't, glot creates it as a copy of the source (references, images, slugs and settings kept) with `language` set, and links it in the metadata document. Strings that look like identifiers (`layout: "grid"`, URLs, paths) and fields such as `slug`, `href`, `url`, `color` and `variant` are skipped. `settings.fields` (`["title", "body"]`) lists the fields to translate when the guess isn't right.
- **Portable Text**: only `span.text` is translated. `marks`, `markDefs` (links, annotations), `_key`s, styles and custom blocks (images, code, embeds) are kept, and spans with the `code` decorator are not translated.
- **Drafts**: glot reads published documents. It writes the published document, and its draft (`drafts.…`) too when there is one, so an editor's next publish doesn't erase the translation. `settings.types` (`["post", "page"]`) limits the document types.
- **Locales**: `settings.locales` lists them. Otherwise glot finds them from the `language` values (document-level) or the keys of the localized objects (field-level).
- **Webhooks**: sanity.io/manage → API → Webhooks → Create a [GROQ-powered webhook](https://www.sanity.io/docs/webhooks) on the card's webhook URL, trigger on create and update, filter for example `_type in ["post", "page"]`. Set a **secret** and paste it in **Webhook secret**: glot checks the `sanity-webhook-signature` header (`t=…,v1=…`, HMAC-SHA256 of `timestamp.body`, base64url). Without a secret, the token in the URL authenticates the call. Each call starts an incremental sync (`_updatedAt`).
- **Limits**: field-level i18n with one document per locale *and* localized objects at once is not supported (pick a mode). Releases (`versions.…`) are not read.

## Strapi

Dashboard → **Integrations** → Strapi, for Strapi **4** and **5** with the [i18n plugin](https://docs.strapi.io/cms/features/internationalization) (built into Strapi 5). glot uses the REST API ([v5 locales](https://docs.strapi.io/cms/api/rest/locale), [v4 i18n](https://docs-v4.strapi.io/dev-docs/plugins/i18n)). The version is detected from the responses.

- **Credentials**: the server **URL** and an **API token** (Settings → API Tokens). *Full access* is simplest. A custom token needs `find`, `findOne`, `update` and `createLocalization` on the translated types, `find` on i18n locales and, to discover the schema, read on the content-type-builder.
- **What is translated**: every collection and single type with localization enabled, and in it the localized attributes of type Text (`string`, `text`), Rich text (Markdown, `richtext`) and Rich text (Blocks, `blocks`, Strapi 5). glot finds them through the content-type-builder. Without access to it, list the types in `settings.content_types` (`["articles", "single:homepage"]`, plural API ids) and glot guesses the text fields. Values are read in the default locale (`GET /api/i18n/locales`).
- **Structure kept**: Markdown is translated line by line, list item by list item and cell by cell. The syntax, links and code blocks are untouched. In Blocks, only `text` nodes are translated: links, bold and italic stay on their nodes, and code blocks and inline code are not translated. Components, dynamic zones, UIDs (slugs), media and relations are left alone.
- **Push**: Strapi 5: `PUT /api/{plural}/{documentId}?locale=fr`, which creates the locale or updates it. Strapi 4: `PUT /api/{plural}/{id}` on the existing localization, or `POST /api/{plural}/{id}/localizations` to create it. Only localized fields are sent. Strapi 5 writes the locale as a draft (`status=draft`) unless you set `{"settings": {"publish": true}}`. Strapi 4 creates localizations the way its REST API does by default, and with `publish` glot also sets `publishedAt`.
- **Webhooks**: Settings → Webhooks → Create new webhook, the card's webhook URL (it carries its token), events *Entry: create, update, publish*. Or use the URL without `?token=` and add a header `Authorization: Bearer <token>`. glot also accepts the signed form from Strapi's docs, `X-Webhook-Signature` (HMAC-SHA256 of `X-Webhook-Timestamp.body`). Each call starts an incremental sync (`filters[updatedAt][$gte]`).
- **Limits**: text inside components and dynamic zones isn't synced yet. In Strapi 4, rich text is Markdown only (there are no Blocks).

## WooCommerce

Dashboard → **Integrations** → WooCommerce. glot uses the [REST API `wc/v3`](https://developer.woocommerce.com/docs/apis/rest-api/v3/products/) with a REST API key ([HTTP Basic auth](https://woocommerce.github.io/woocommerce-rest-api-docs/#authentication), so the store must be on HTTPS). WooCommerce stores no translations of its own: as with WordPress, glot writes them into the site's multilingual plugin.

- **Credentials**: WooCommerce → Settings → Advanced → REST API → Add key, permission **Read/Write**, for a Shop manager or an Administrator. Copy the **consumer key** (`ck_…`) and **consumer secret** (`cs_…`). The **webhook secret** is optional: with it, glot registers webhooks and verifies them.
- **What is translated**: products (name, short description, description), product categories and tags (name, description). Values are read raw (`context=edit`). HTML descriptions keep their tags, attributes and shortcodes, and only the text between tags is translated. SKUs, slugs, prices, attributes and variations are not touched: WooCommerce Multilingual syncs variations from the translated product.
- **Multilingual plugin**: detected from the REST index (`settings.plugin: "polylang" | "wpml"` forces one). glot pulls only the source language (`?lang=en`).
  - **WPML + WooCommerce Multilingual**: existing translations come from the `translations` map of each product or term. A missing one is created with `POST /wc/v3/products` and `"lang": "fr", "translation_of": 12` in the body, as [WPML documents it](https://wpml.org/documentation/related-projects/woocommerce-multilingual/using-wordpress-rest-api-woocommerce-multilingual/). WPML has no languages endpoint: set `settings.languages` and `settings.source_lang`.
  - **Polylang for WooCommerce** (with Polylang Pro for the REST API): languages come from `/pll/v1/languages`, and a missing translation is created with `?lang=fr&translations[en]=12`. Polylang documents this linking for `wp/v2` only. If your version ignores it on products, create the translation once in Polylang, and glot keeps it updated from then on.
  - **No plugin**: glot can read and translate, but pushes fail with a message saying there is nowhere to write.
- **New products** are created as `draft`, with the source's type and regular price, so you review them in WooCommerce. Set `settings.new_status` to `publish`, or to `same` to copy the source's status. A new category or tag needs its translated name.
- **Webhooks**: with a webhook secret (any long random string), glot subscribes [`product.created` and `product.updated`](https://developer.woocommerce.com/docs/apis/rest-api/v3/webhooks/) to the card's webhook URL and checks `X-WC-Webhook-Signature` (base64 HMAC-SHA256 of the body). Products are then re-read with `modified_after`. Categories and tags have no webhook topic: **Sync now** or the periodic push picks them up. Without a secret, add the webhooks yourself in WooCommerce → Settings → Advanced → Webhooks with the card's URL (it carries its token).
- **Rate limits**: the REST API has no rate limit of its own. glot waits and retries on `429`/`503` from hosts or security plugins.
- **Limits**: hosts that strip the `Authorization` header block Basic auth: allow it in your server configuration. Product attributes, variation descriptions, custom fields and page-builder content aren't synced.

## BigCommerce

Dashboard → **Integrations** → BigCommerce. glot reads the catalog with the REST API v3 and writes translations with the [Translations Admin GraphQL API](https://docs.bigcommerce.com/developer/docs/admin/store-configuration/translations/overview) (`translation.updateTranslations`), per channel and locale.

- **Credentials**: in the BigCommerce control panel → Settings → Store-level API accounts, create an API account with **Products** (modify), **Store content/Information** (read-only), **Channel settings** (modify, for locales) and, for webhooks, the scopes they need. Copy the **access token** and the **store hash** (the `abc123` in `https://api.bigcommerce.com/stores/abc123/v3/`). **Channel ID** is `1` (the default storefront) unless you translate another channel. **Webhook secret** is optional: any random string. With it, glot registers webhooks.
- **What is translated**: products (`name`, `description`, `page_title`, `meta_description`, `search_keywords`), categories (same fields, from `/v3/catalog/trees/categories`) and brands (`name`, `page_title`, `meta_description`, `search_keywords`). Descriptions are HTML: only the text between tags is translated. SKUs, URLs, images, custom fields, options and `meta_keywords` are left alone. Web pages (`/v3/content/pages`) have no translation API at BigCommerce and are not synced.
- **Locales**: glot reads the channel's locales (`store.locales`). Their default locale is the source, and BigCommerce takes no translations for it. A glot target locale missing from the channel is added with `addLocale`, **inactive**: you activate it in BigCommerce (Settings → Languages) when you're ready.
- **Rate limits**: glot follows [BigCommerce's limits](https://docs.bigcommerce.com/docs/start/best-practices/api-rate-limits). On `429` it waits `X-Rate-Limit-Time-Reset-Ms` and retries. When `X-Rate-Limit-Requests-Left` reaches 0, it waits for the window to reset before the next call.
- **Webhooks**: BigCommerce doesn't sign its callbacks. With a **Webhook secret**, glot subscribes `store/product/created`, `store/product/updated`, `store/category/created` and `store/category/updated` to the card's webhook URL, sending the secret in an `X-Glot-Webhook-Secret` header, and checks it on each call. BigCommerce only calls HTTPS URLs on port 443. Each call starts an incremental sync: products changed since the last sync (`date_modified:min`), plus categories and brands.
- **Limits**: brands have no change webhook, so brand edits are picked up by **Sync now** or the next sync. If BigCommerce rejects one field, it still saves the others, but glot marks the whole batch for that resource type as failed and pushes it again on the next sync. Pre-order messages, warranty, availability text and image alt text are not synced yet.

## PrestaShop

Dashboard → **Integrations** → PrestaShop, for PrestaShop 1.7, 8 and 9. glot uses the [Webservice API](https://devdocs.prestashop-project.org/8/webservice/getting-started/) (`/api/…`) and reads JSON (`output_format=JSON&display=full`).

- **Credentials**: the shop **URL** and a **webservice key**. In the back office, go to Advanced Parameters → Webservice, enable the webservice, then add a key with `GET` and `PUT` on `products`, `categories`, `content_management_system` and `manufacturers`, and `GET` on `languages` and `configurations`. glot sends the key as the HTTP Basic user name with an empty password. Without access to `configurations`, glot can't read the shop name or default language: set `settings.source_lang` (`"en"`).
- **What is translated**: products (name, short description, description, meta title, meta description), categories (name, description, meta title and description), CMS pages (meta title, SEO title, meta description, content) and brands (short description, description, meta title and description). Values are read in the shop's default language (`PS_LANG_DEFAULT`). Descriptions and CMS content are HTML: glot translates the text and keeps every tag. URL slugs (`link_rewrite`), keywords, features and attribute values are not touched. The invisible root category is skipped.
- **Push**: the Webservice only reads XML, and partial updates (`PATCH`) exist only from PrestaShop 8. So glot reads each resource as XML, replaces only the target language's `<language id="…">` node of each translated field, and writes the document back with `PUT`. Other languages, prices, stock and associations are sent back unchanged. The read-only fields (`manufacturer_name`, `quantity`, `level_depth`, `nb_products_recursive`) are left out of the PUT, as the docs require.
- **Languages**: a glot target locale is matched to the shop language with the same `language_code` (`pt-br`), else the same `iso_code` (`fr` ↔ `fr-FR`). Add and enable the language in International → Localization → Languages first. If no language matches, the push for that locale fails with a message saying so.
- **Webhooks**: PrestaShop has none. A module (an `actionObjectProductUpdateAfter` hook, for example) can call the card's webhook URL, which carries its token. It can instead send `X-Glot-Webhook-Secret: <token>`, or sign the body with `X-Glot-Signature: sha256=<hex HMAC-SHA256>` using the token as key. Each call starts an incremental sync (`filter[date_upd]` on products, categories and brands; CMS pages are always read in full).
- **Rate limits**: the Webservice documents none; glot waits and retries on `429`/`503`.
- **Limits**: multistore shops are synced in the default shop's context. Product features, attribute values, image legends and theme strings are not synced. Translate theme strings in their files, or with the [glot script tag](/docs/edge#in-the-browser-with-one-script-tag).

## Magento / Adobe Commerce

Dashboard → **Integrations** → Magento / Adobe Commerce, for Magento Open Source and Adobe Commerce on-premises or on cloud infrastructure (PaaS). glot uses the [REST API](https://developer.adobe.com/commerce/webapi/rest/use-rest/performing-searches/) and writes each language into its **store view**, so create one store view per language first (Stores → All Stores) and set its locale (Stores → Configuration → General → Locale Options).

- **Credentials**: in the admin, System → Extensions → Integrations → Add New Integration, give it the Catalog resources (Products, Categories) and Stores → Settings → All Stores, then save and activate it. Paste the store **URL**, the **access token**, and also the **access token secret**, **consumer key** and **consumer secret**: glot then signs every call with [OAuth 1.0a](https://developer.adobe.com/commerce/webapi/get-started/authentication/gs-authentication-oauth/) (HMAC-SHA256). With the access token alone, glot sends it as a Bearer token. Commerce 2.4.4 and later refuse that unless *Allow OAuth Access Tokens to be used as standalone Bearer tokens* is set to Yes (Stores → Configuration → Services → OAuth → Consumer Settings, [not recommended by Adobe](https://developer.adobe.com/commerce/webapi/get-started/authentication/gs-authentication-token)).
- **What is translated**: products (name, description, short description, meta title, keywords and description) and categories below the root (name, description, meta fields). Values are read in the source store view, `default` unless you set `settings.source_store`. Descriptions are HTML (Page Builder included): only the text between tags is translated. SKUs, URL keys, prices, attributes and images are never read or written. `settings.resources` (`["product"]`) limits the list.
- **Store views**: glot reads them with `GET /V1/store/storeConfigs`. A glot locale is written to every store view whose code equals it, whose locale equals it (`fr-FR` ↔ `fr_FR`) or, for a locale without a region, whose locale has the same language (`fr` → `fr_FR` and `fr_CA`). The source store view is never written. Use `settings.locale_map` (`{"fr": "french_store"}`) to pick one.
- **Push**: `PUT /rest/{store_code}/V1/products/{sku}` and `PUT /rest/{store_code}/V1/categories/{id}`, with only the translated attributes. Only those fields get a store-view value. Everything else keeps *Use Default Value* and goes on inheriting.
- **Webhooks**: Magento Open Source has none, and Adobe Commerce [webhooks](https://developer.adobe.com/commerce/extensibility/webhooks/) are synchronous calls that block the save. Make an extension or a cron job call the card's webhook URL (it carries its token), or sign the body with `X-Glot-Signature: sha256=<hex HMAC-SHA256>` using the token as key. Each call starts an incremental sync (`updated_at` ≥ the last sync).
- **Rate limits**: rate limiting is [off by default](https://developer.adobe.com/commerce/webapi/get-started/rate-limiting); when Fastly or the store returns `429`, glot waits and retries.
- **Limits**: CMS pages and blocks are not synced: each one is assigned to store views as a whole (one page per language), so there is no per-store-view value to write into. Adobe Commerce as a Cloud Service (SaaS) uses IMS authentication and is not supported yet. SKUs containing `/` can fail on some web servers even when encoded.

## Shopware

Dashboard → **Integrations** → Shopware 6. glot uses the [Admin API](https://developer.shopware.com/docs/guides/development/integrations-api/auth-api-requests.html) with an integration: it gets a short-lived OAuth token (`POST /api/oauth/token`, `client_credentials`) and sends it as a Bearer token.

- **Credentials**: in the Shopware Administration → Settings → System → Integrations, add an integration with the **Administrator** role (or one that can read and write products, categories and properties, and read languages). Copy the **Access key ID** (`SWIA…`) and the **Secret access key** (shown once), and give the shop URL (`https://shop.example.com`).
- **What is translated**: products (name, description, meta title, meta description, keywords), categories (name, description, meta fields), property groups (name, description) and their options (name). Only main products are read: variants inherit their parent's texts. Descriptions are HTML and keep their tags. Product numbers, URLs (SEO URLs), media, custom fields and Shopping Experiences (CMS) layouts are left alone.
- **Pull**: `POST /api/search/{entity}`, 100 per page, in the shop's default language (no `sw-language-id` header), which glot compares with the project's source language. An incremental sync reads only what was created or updated since the last sync (`updatedAt` / `createdAt` range filter).
- **Push**: `PATCH /api/{entity}/{id}` with the translations association indexed by language id (`{"translations": {"<languageId>": {"name": "…"}}}`), so only that language's translation is written. The default language is never written. Shopware's docs describe this payload in a [decision record](https://developer.shopware.com/docs/resources/references/adr/2022-03-29-specify-priority-of-translations-in-dal-write-payloads.html), not in a guide: check the first push on a staging shop.
- **Locales**: glot reads the shop's languages (`POST /api/search/language` with their locale) and matches each glot target locale to a locale code: exact codes first, then the same language (`de` ↔ `de-DE`). Add missing languages in Settings → Languages and assign them to your sales channel, or map codes with `settings.locale_map` (`{"pt": "pt-PT"}`).
- **Webhooks**: Shopware has no API for registering a plain webhook. With Flow Builder's [**Call webhook** action](https://docs.shopware.com/en/shopware-6-en/features/webhook-actions-in-flow-builder) (Shopware Evolve and Beyond), add a flow on product and category changes that calls the card's webhook URL (it carries its token), or send the header `X-Glot-Webhook-Secret` with the token's value. If a Shopware app forwards its webhooks, put the app secret in **Webhook secret**: glot then checks `shopware-shop-signature` (hex HMAC-SHA256 of the body). Each call starts an incremental sync.
- **Rate limits**: the Admin API has no documented rate limit; glot waits and retries on `429`/`503`.
- **Limits**: text in Shopping Experiences (CMS slots), variant-specific texts, manufacturers and custom fields isn't synced yet.

## Square Online

No connector: Square's APIs have nothing to store translations in. The [Sites API](https://developer.squareup.com/docs/sites-api/overview) only returns basic site details (id, title, domain), the Catalog API has no per-language fields, and Square Online has no multilingual content of its own. Translate the storefront with glot Edge instead:

- **Script tag**: Square Dashboard → Channels → Square Online → Settings → **Tracking Tools** → Add new code, placement **head**, paste the [glot script tag](/docs/edge#in-the-browser-with-one-script-tag), save, then publish the site. Square lists this for its Free, Plus and Premium plans ([help](https://squareup.com/help/us/en/article/6957)). Square's own [Snippets API](https://developer.squareup.com/docs/snippets-api/overview) injects code the same way (end of `<head>`, every page except checkout), but it is for approved App Marketplace apps, so glot does not use it.
- **Subdomain** (`es.shop.example.com`, [setup](/docs/edge#subdomain-hosted-by-glot)): needs your own domain on the site, and Square only connects a custom domain on a paid plan. The free plan (`*.square.site`) can only use the script tag.
- **Limits**: checkout pages are Square's and are never translated (no code runs there). Product data from the catalog is translated as it is displayed, not stored in Square.

## Zoho Commerce

No connector: Zoho says Zoho Commerce [has no built-in multilingual feature](https://www.zoho.com/commerce/kb/online-store/store-in-multiple-languages/) and leaves languages to third-party translators, so its API has no per-language fields to write into. Use glot Edge:

- **Script tag**: in the theme editor, add a [Custom Code](https://www.zoho.com/commerce/help/theme-editor/custom-code/) section (Liquid, HTML and JavaScript allowed) containing the [glot script tag](/docs/edge#in-the-browser-with-one-script-tag), make it available in the header group (`"header-group": true`) and on every page type (`available_in.pages`), and put it in the header. Zoho documents no site-wide "header code" field for Commerce, so check that the section shows on product and cart pages too.
- **Subdomain** (`es.shop.example.com`, [setup](/docs/edge#subdomain-hosted-by-glot)): needs the store on your own domain, with access to its DNS.
- **Limits**: content is translated as it is displayed, not stored in Zoho; Zoho-hosted checkout is not covered by the script if your section isn't rendered there.

## HubSpot CMS

Dashboard → **Integrations** → HubSpot CMS. glot uses the CMS API v3 ([blog posts](https://developers.hubspot.com/docs/api-reference/latest/cms/blogs/posts/guide), [pages](https://developers.hubspot.com/docs/api-reference/cms-pages-v3/guide)) and writes each language as a HubSpot **multi-language variation** of its source, so HubSpot's language switcher, `hreflang` tags and language groups keep working.

- **Token**: Settings → Integrations → Private apps → Create, scope `content`, then copy the access token (`pat-…`). The **client secret** is optional (webhook signatures).
- **What is translated**: blog posts (`name`, `htmlTitle`, `metaDescription`, `postSummary`, `postBody`), website pages and landing pages (`htmlTitle`, `metaDescription`, and every `html` value of their modules: rich text modules in `widgets` and in drag-and-drop areas `layoutSections`). Module HTML keeps its tags. Images, CSS classes and the other module parameters are left alone. Variations (`translatedFromId` set) are never read as sources. `settings.resources` (`["blog_posts", "site_pages", "landing_pages"]`) limits the list.
- **Push**: glot reads the source's `translations` map. If the language has a variation, glot writes into it. If it doesn't, glot creates one with `POST …/multi-language/create-language-variation` (`{"id", "language"}`). Values go to the variation's **draft** (`PATCH …/{id}/draft`). With `{"settings": {"publish": true}}`, glot then calls `…/draft/push-live`. If you moved modules around in a variation, a module that's no longer there fails with a message naming it, and the rest of the page is still written.
- **Languages**: HubSpot needs a language on the source content (page or post settings → Language). glot lists the languages in use. Set `settings.languages` before the first variation exists.
- **Webhooks**: private apps get no CMS content events. Use a workflow *Send a webhook* action on the card URL (it carries its token), or **Sync now**. Calls signed with `X-HubSpot-Signature` v1 (SHA-256 of client secret + body) are accepted when the client secret is set. Incremental syncs read with `updatedAfter`.
- **Limits**: private apps get 100 requests per 10 s (Free, Starter) or 190 (Professional, Enterprise). glot waits on `429`. HubDB tables, theme fields, global modules and email are not synced. HubSpot now documents dated paths (`/cms/blogs/2026-09/posts`) and keeps v3 working; glot uses v3.

## Drupal

Dashboard → **Integrations** → Drupal, for Drupal 10 and 11 with the core **JSON:API**, **Content Translation** and **Language** modules.

- **Credentials**: the site **URL** and either a **username and password** (core `basic_auth` module) or an **OAuth token** ([Simple OAuth](https://www.drupal.org/project/simple_oauth)). Use an editor account allowed to translate content. JSON:API must accept writes: `/admin/config/services/jsonapi` → *Accept all JSON:API create, read, update, and delete operations*.
- **What is translated**: nodes and taxonomy terms of every bundle in the JSON:API index (`settings.resources`: `["node--article", "taxonomy_term--tags"]` limits it). Fields: `title` / `name` and every formatted text field (`body`, text long), `value` and `summary`, as HTML (tags kept, text format kept). Paths, links, references and plain string fields are left alone.
- **Push**: `PATCH /{langcode}/jsonapi/{entity}/{bundle}/{uuid}`. The language prefix selects the translation, so language negotiation by URL prefix must be on (the default). Drupal's core JSON:API [cannot create a translation](https://www.drupal.org/docs/core-modules-and-themes/core-modules/jsonapi-module/translations), only update one. glot first reads the translation. If Drupal answers in another language (no translation yet), glot reports it and doesn't write anything. Add the translation once (Translate tab → Add), and glot keeps it updated from then on. The contrib [JSON:API Multilingual](https://www.drupal.org/project/jsonapi_multilingual) module lifts this limit, but glot doesn't use it yet.
- **Languages**: read from `configurable_language` (Drupal language codes). Map glot locales with `settings.locale_map` (`{"pt-BR": "pt-br"}`) when they differ.
- **Webhooks**: Drupal sends none. A custom module or [ECA](https://www.drupal.org/project/eca) can call the card URL (it carries its token), or send `X-Glot-Webhook-Secret`. Each call starts a sync.
- **Limits**: Paragraphs, Layout Builder blocks, menus and blocks are not synced.

## Craft CMS

Dashboard → **Integrations** → Craft CMS, for Craft 4 and 5 [GraphQL](https://craftcms.com/docs/5.x/development/graphql.html). Craft translates with **sites**: one site per language, and fields set to translate per site.

- **Credentials**: the **GraphQL endpoint** (a route to `graphql/api`, often `/api`) and a **schema token** (GraphQL → Tokens). The schema must be able to query the sections and **save** their entries in every site. **Sites per locale** maps glot locales to site handles (`fr: french, de: german`). Without it, glot uses the site whose handle is the locale (`fr`, `fr_CA`).
- **What is translated**: entries of every section (`settings.sections`: `["blog"]` limits it) in the primary site (`settings.source_site` picks another one). For each entry type, glot asks GraphQL which fields are text (`String`) and skips system fields (`slug`, `uri`, `url`…). Plain Text fields are plain. CKEditor and Redactor values are HTML, and keep their tags. Matrix, Neo, relations and assets are left alone.
- **Push**: `save_{section}_{type}_Entry(id, siteId, title, …)` with the `siteId` of the entry in the target site. If the entry isn't enabled in that site, glot reports it: enable the section in the site (Settings → Sections → Site settings) first.
- **Webhooks**: Craft has none built in. The first-party [Webhooks plugin](https://plugins.craftcms.com/webhooks) can call the card URL (it carries its token) on *After save entry*, or send `X-Glot-Webhook-Secret`.
- **Limits**: a field set to *Not translatable* is shared by all sites, so writing it would change the source too. Leave such fields out of the schema. Craft 4 can't list sites through GraphQL, so glot only knows the sites you map.

## Ghost

Dashboard → **Integrations** → Ghost (Ghost 5 and later, [Admin API](https://docs.ghost.org/admin-api)). Ghost has no translations of its own. glot uses the pattern from Ghost's [routing docs](https://docs.ghost.org/themes/routing): one post per language with an **internal tag** (`#fr`), served by a collection filtered on that tag.

- **Credentials**: the site **URL** and an **Admin API key** (Settings → Integrations → Add custom integration, `id:secret`). glot signs a 5-minute token with it for each call. **Webhook secret** is optional: any string you choose. With it, glot registers signed Ghost webhooks.
- **Sources**: posts and pages with no language tag, or only those with `settings.source_tag` (`"#en"`). A post tagged `#fr`, `#pt-br`… is a translation and is never read as a source. Tags such as `#featured` aren't languages.
- **What is translated**: `title`, `custom_excerpt`, `meta_title`, `meta_description`, the `og_*` and `twitter_*` texts, and the body (Lexical): text nodes only. Bold, links, headings, lists and cards (images, embeds, HTML and Markdown cards, buttons) stay in place, and inline code is not translated.
- **Push**: the translation of `my-post` is the post with slug `my-post-fr`. glot updates it with its `updated_at` (Ghost's collision check), or creates it as a **draft** with the source's tags plus `#fr` (`{"settings": {"publish": true}}` creates it published). Slugs, feature images and authors are not changed once the post exists.
- **Routing**: add one collection per language to `routes.yaml`, so `/fr/` lists the French posts and the default collection hides them:

```yaml
collections:
  /fr/:
    permalink: /fr/{slug}/
    filter: 'tag:hash-fr'
  /:
    permalink: /{slug}/
    filter: 'tag:-hash-fr'
```

- **Webhooks**: with a webhook secret, glot creates `post.added`, `post.edited`, `page.added` and `page.edited` webhooks, and checks `X-Ghost-Signature` (`sha256=` hex HMAC-SHA256 of the body followed by `t`). Without one, add a webhook on the card URL (it carries its token) under the custom integration.
- **Limits**: Ghost's theme strings (`{{t}}` helpers) live in the theme's `locales/*.json`. Translate them in Git with the CLI. Tags and newsletters aren't translated. Card captions and button labels inside the body aren't translated yet.

## Zendesk Guide

Dashboard → **Integrations** → Zendesk Guide. glot uses the [Help Center API](https://developer.zendesk.com/api-reference/help_center/help-center-api/translations/): translations are stored by Zendesk itself, one per locale, for articles, sections and categories.

- **Credentials**: the **subdomain** (`acme` for `acme.zendesk.com`), the **email** of an agent who can manage Guide, and an **API token** (Admin Center → Apps and integrations → Zendesk API). The **webhook signing secret** is optional.
- **What is translated**: articles (`title`, `body` as HTML, tags kept), sections and categories (`name`, `description`), read in the Help Center's default locale with cursor pagination. Articles written in another source locale are skipped.
- **Push**: `PUT /api/v2/help_center/{articles|sections|categories}/{id}/translations/{locale}`, or `POST …/translations` when the locale doesn't exist yet. New article translations are **drafts** (`{"settings": {"publish": true}}` publishes them). Existing ones keep their draft or published state. Zendesk needs a title to create a translation, so an article whose title isn't translated yet waits.
- **Locales**: the Help Center's locales (Guide admin → Settings → Language settings). glot matches `fr-FR` to `fr`. A glot locale that isn't enabled in Guide fails with a message saying so.
- **Webhooks**: Admin Center → Apps and integrations → Webhooks, endpoint = the card URL without `?token=`, subscribed to the Help Center article events. Paste its signing secret in **Webhook signing secret**: glot checks `X-Zendesk-Webhook-Signature` (base64 HMAC-SHA256 of timestamp + body, [docs](https://developer.zendesk.com/documentation/webhooks/verifying/)). The card URL with its token works too.
- **Limits**: 200 to 2,500 requests per minute depending on the plan. glot waits on `429`. Article attachments, labels and Guide theme strings aren't translated.

## Help Scout Docs

Dashboard → **Integrations** → Help Scout Docs. Docs has no languages: Help Scout's [own recommendation](https://docs.helpscout.com/article/302-translations) is one Docs site per language. glot reads the source site and writes every language into its own site. A translation is linked to its source by the **slug**, which is the same in both sites.

- **Credentials**: an **API key** (Your Profile → Authentication → API Keys). **Collections per locale** maps each glot locale to a collection of that language's site (`fr: 5f2a…, de: 61b0…`). The collection id is in the URL of its page in Help Scout.
- **What is translated**: articles (`name` and `text` as HTML, tags kept) of every collection that isn't a target (`settings.source_site` restricts sources to one site).
- **Push**: `PUT /v1/articles/{id}` on the article with the same slug in the target collection, otherwise `POST /v1/articles` (created `notpublished`, `{"settings": {"publish": true}}` publishes). Categories, related articles and keywords aren't copied.
- **Webhooks**: Help Scout has none for Docs. Use **Sync now** (the periodic push still sends new reviews every 10 minutes), or call the card URL from your own tooling.
- **Limits**: 2,000 requests per 10 minutes for one site ([API](https://developer.helpscout.com/docs-api/)). glot reads every article once per full sync, so a first sync of a large site takes a while. Beacon and site settings aren't translated.

## Wix, Squarespace, TYPO3 and course platforms

These have no API glot can write translations into, so there's no connector. Translate them with the [website tag](/docs/website#where-to-paste-it), plus the [subdomain mode](/docs/edge#subdomain-hosted-by-glot) for pages you want indexed in other languages.

| Platform | Why no connector | Tag | Subdomain |
|---|---|---|---|
| Wix, Wix Studio | Wix Multilingual has a [Translation Content API](https://dev.wix.com/docs/api-reference/business-management/multilingual/translation/translation-content/introduction), but its entries are keyed by schemas that each Wix app defines. The docs don't say which content (pages, blog, store) it exposes, or whether a site API key can reach it. | Settings → Custom Code, Head (site published with a connected domain) | CNAME on your own domain |
| Squarespace | Only Commerce, Webhooks and Reseller APIs: no pages or blog API ([developers](https://developers.squarespace.com/)). | Code Injection, Header (Core, Plus, Advanced) | Your domain's DNS |
| TYPO3 | No core API to write content. [EXT:headless](https://docs.typo3.org/p/friendsoftypo3/headless/main/en-us/Index.html) only renders pages as JSON. TYPO3's own localization (one record per language) stays the way to store translations in TYPO3. | Add the tag to the site package's `page.headerData` (TypoScript) | Your domain's DNS |
| Thinkific | The Admin API reads courses, chapters and lessons ([spec](https://developers.thinkific.com/openapi/thinkific-admin-api-v1.yaml)) but can't write them, and has no locale. | Site footer code (paid plans), head with theme code access | CNAME, paid plans |
| Teachable | Course and lecture endpoints are read-only ([API](https://docs.teachable.com/docs/authentication)). | Code snippets, from the Builder plan (not on sso.teachable.com) | CNAME, depends on the plan |
| Kajabi | Blog posts, courses and pages are read-only in the [API](https://developers.kajabi.com/). | Custom code on pages or site-wide | CNAME for subdomains |
| Podia | No public API ([Podia](https://help.podia.com/en/articles/517)). | Third-party code before `</body>` only, on plans with custom code | `www` only |
| LearnWorlds | The API's write support for course content and any per-language field aren't publicly documented. | Custom code from Starter | Custom domain from Pro |

The tag translates what the browser shows, including course pages, but it can't translate what the platform renders outside your pages: checkout, emails, video players, PDFs.
