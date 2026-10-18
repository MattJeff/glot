# @glot-i18n/overlay

Edit translations **directly in your running app**, with AI. Alt+click any text → a popover shows the key, its value in every locale (editable inline), live validator issues, AI actions, and whether the element overflows its container. Saving writes your catalog files through `glot dev`, so your app's HMR reloads instantly.

Dev only, ~14 kB minified (~6 kB gzip), zero runtime dependencies, styles isolated in a Shadow DOM, light/dark.

| Alt+click → popover (light) | AI “Shorten” on the French page (dark) |
|---|---|
| ![popover](https://raw.githubusercontent.com/MattJeff/glot/main/packages/overlay/docs/popover-light.png) | ![AI suggestions](https://raw.githubusercontent.com/MattJeff/glot/main/packages/overlay/docs/popover-dark-ai.png) |

![inspect mode](https://raw.githubusercontent.com/MattJeff/glot/main/packages/overlay/docs/inspect.png)

<sub>Captured on a copy of the glot dashboard (next-intl, 6 locales) running `next dev` + `glot dev` with the mock provider — the `⟦fr⟧` prefix is the mock's fake output.</sub>

## Start

```sh
glot dev            # in the project holding glot.toml → prints a token
```

Then load the overlay in your app, in dev only. Either:

**Next.js (App Router), 3 lines** in `app/layout.tsx`:

```tsx
import { GlotOverlay } from "@glot-i18n/overlay/react";
// inside <body>:
{process.env.NODE_ENV === "development" && <GlotOverlay />}
```

**Any app, no install**: `glot dev` serves the bundle itself:

```html
<script src="http://127.0.0.1:4317/overlay.js" defer></script>
```

**Bundler**: `if (import.meta.env.DEV) import("@glot-i18n/overlay");`

The first time, open your app with `?glot_token=<token>` (stored in `localStorage`) or paste the token when the overlay asks. Pin it with `glot dev --token …` / `GLOT_DEV_TOKEN` and pass `<GlotOverlay token={process.env.NEXT_PUBLIC_GLOT_DEV_TOKEN} />` if you prefer.

## Use

| | |
|---|---|
| **Alt+click** a text | open the popover for it |
| **Alt+Shift+G** | inspect mode: hover highlights, click selects, Esc quits |
| **Enter** in a field | save (validated; rejected edits are not written) · Shift+Enter = newline · Esc = revert |
| **view** next to a locale | switch the app to it (`NEXT_LOCALE` cookie, or `/fr/…` prefix) and reload |
| **Suggest / Shorten / More formal** | AI rewrites of the focused locale, guarded by the validator (placeholders, glossary) |
| **Translate all** | fill missing/stale locales of this key with your configured LLM |
| Text without key | **Extract to key**: proposes a key (from `glot extract`), adds it to the source catalog, shows where the literal is in your code |

Lookup is placeholder-tolerant (`Hello {name}!` matches “Hello Ana!”, ICU plurals and rich tags included) and works whatever locale the app currently shows.

## Options

```ts
init({ url: "http://127.0.0.1:4317", token: "…", localeCookie: "NEXT_LOCALE" });
```

Set `window.GLOT_OVERLAY_MANUAL = true` before importing to skip auto-start. With `NODE_ENV=production` it never starts.

## `glot dev`

`glot dev [--port 4317] [--origin http://localhost:3000] [--token …]` — binds `127.0.0.1` only, requires the bearer token on every API call, CORS restricted to the given origin(s) (default: `localhost`/`127.0.0.1` origins). Polls catalog files and pushes `change` events (SSE) so open popovers refresh.

| Route | |
|---|---|
| `POST /api/lookup {text}` | `{matches:[{key,locale,value,score}]}` |
| `GET /api/key?key=` | values per locale, issues, lock entries, note, code refs |
| `POST /api/set {key,locale,value}` | validated write → `{ok,issues}` |
| `POST /api/suggest {key|text,locale,instruction}` | `{suggestions:[{value,issues}]}` |
| `POST /api/translate {key,force?}` | translate report |
| `POST /api/propose {text}` · `POST /api/extract {key,value,note?}` | hardcoded text → new source key |
| `GET /api/events?token=` | SSE `change` |
| `GET /overlay.js` | this bundle |

## Develop

```sh
pnpm install && pnpm build
cargo build -p glot-cli && pnpm test   # Playwright e2e against `glot dev` (mock provider)
```
