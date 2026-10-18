# Frameworks

`glot i18nize` rewrites hardcoded strings and wires the i18n runtime it detects. The setup is idempotent (a second run changes nothing) and every edit shows up in `--dry-run`. Use `--no-setup` to only rewrite code, `--lib next-intl|react-i18next` to force the target library.

Each setup below is verified end to end on a real app in [`examples/frameworks/`](https://github.com/MattJeff/glot/tree/main/examples/frameworks).

| Stack | Library | What `i18nize` writes |
|---|---|---|
| Next.js App Router | next-intl | `i18n/request.ts`, `i18n/routing.ts`, `proxy.ts` (or `middleware.ts`), the plugin in `next.config`, the provider in `app/layout.tsx` |
| Next.js Pages Router | next-intl | Built-in `i18n` config and a provider wrapping the whole `_app` tree |
| React Router 7/8 (Remix) | remix-i18next | Middleware, `entry.server.tsx` with `I18nextProvider`, dynamic `<html lang>` |
| Astro (React islands) | react-i18next | Native i18n routing, `src/middleware.ts` |
| Vite + React | react-i18next | An init module imported from the entry |

## Next.js App Router

```sh
glot i18nize --yes                      # cookie routing (default)
glot i18nize --yes --routing prefix     # non-default locales under /fr/...
```

- `--routing cookie` (default): `/fr/...` selects the locale and stores it in a cookie, unprefixed URLs keep it. Your `app/` tree does not move under `app/[locale]`.
- `--routing prefix`: non-default locales always live under `/fr/...`; the default locale has no prefix (`as-needed`).

Server and Client Components both use next-intl (`getTranslations` / `useTranslations`). Module-level config arrays with UI copy (navigation, plans, FAQ) become hooks so their text can be translated at render.

## Next.js Pages Router

Uses Next's built-in `i18n` routing (`/fr/...`) and wraps `_app` in the provider, so every page and component gets `useTranslations`.

## React Router 7/8 (Remix)

remix-i18next middleware creates one i18next instance per request. The locale comes from `?lng`, then the `glot-locale` cookie, then `Accept-Language`. `entry.server.tsx` renders with `I18nextProvider` and `<html lang>` follows the locale, so SSR renders the requested language.

## Astro

Native Astro i18n routing with `fallbackType: "rewrite"`: `/fr/...` reuses your pages. `src/middleware.ts` sets the locale before React islands render. The i18next instance is shared: safe for static builds, not for concurrent on-demand rendering.

## Vite + React

A react-i18next init module imported from your entry. This is a client-side SPA setup: the locale is resolved in the browser.

## Other stacks

glot reads and writes catalogs for any library that uses JSON files (`format = "json-nested" | "json-flat"`) with ICU, i18next or MessageFormat 2 syntax. Without `i18nize` support for your framework, use `glot extract` to list hardcoded strings and your coding agent with the [MCP server](mcp.md) to convert them.
