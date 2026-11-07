# Runtime fallback

You ship a handful of locales with `glot translate`. Visitors whose language is not one of them usually get English. `@glot-i18n/runtime` covers that long tail at zero cost: the UI is translated **on the device** with the browser's Translator API (Chrome), with the same guarantees as the CLI.

- Placeholders, rich-text tags and glossary terms are masked before translation and restored after.
- Plural structure is kept.
- Every string is validated by glot-core compiled to WebAssembly. A string that fails stays in the source language: nothing unvalidated is ever shown.
- Shipped locales are never touched.

```sh
npm i @glot-i18n/runtime
```

Serve `node_modules/@glot-i18n/runtime/dist/glot.wasm` (for example as `public/glot.wasm`) and pass `wasm="/glot.wasm"`, or let your bundler resolve it next to the module.

## Plain JS

```ts
import { fallbackWithBadge } from "@glot-i18n/runtime";

await fallbackWithBadge({ source: en, glossary, locale: "de", onMessages: (m) => render(m) });
```

`createFallbackMessages` does the same without the "translated on your device" badge. It never throws and returns a `status`: `translated`, `unsupported`, `needs-activation`, …

## next-intl (App Router)

```tsx
// app/layout.tsx (Server Component)
import { NextIntlClientProvider } from "next-intl";
import { getMessages } from "next-intl/server";
import { GlotFallbackProvider } from "@glot-i18n/runtime/next-intl";

<NextIntlClientProvider>
  <GlotFallbackProvider shipped={["en", "fr"]} source={await getMessages()} glossary={glossary} wasm="/glot.wasm">
    {children}
  </GlotFallbackProvider>
</NextIntlClientProvider>
```

- **Client Components** (`useTranslations`): nothing to change.
- **Server Components**: replace `getTranslations` with `getGlotTranslations`:

```tsx
import { getGlotTranslations } from "@glot-i18n/runtime/next-intl/server";

const t = await getGlotTranslations("home");
return <h1>{t("title", { name })}</h1>;
```

The server HTML stays in the shipped language (SEO, no-JS). After hydration, and only when the browser's language is not shipped, each element swaps to its validated on-device translation. There is no DOM patching, so React sees no hydration mismatch.

Limits: `t()` from `getGlotTranslations` returns elements, so keep next-intl's `t` for attributes (`alt`, `placeholder`), `generateMetadata` and `t.rich`; those stay in the shipped language. Server-rendered unshipped locales (for SEO) are out of scope: ship the locale with `glot translate`.

## i18next

```ts
import { GlotFallbackBackend } from "@glot-i18n/runtime/i18next";
// chain it after your normal backend; use `react: { bindI18nStore: "added" }` to re-render once the model is downloaded
```

## Browser support

The Translator API is available in recent Chrome desktop. Elsewhere the status is `unsupported` and the source language is shown. When the browser needs to download a language model, the status is `needs-activation` and the badge offers a click to start it.

Example and end-to-end test (real Next build, `de` not shipped): [`examples/runtime-next`](https://github.com/MattJeff/glot/tree/main/examples/runtime-next).

## Language switcher & live languages

A drop-in `<glot-locale-switcher>` element, fed by your project's public CDN. It works with any stack (React, Vue, Svelte, Leptos, plain HTML): one script tag, no dependency, ~5 kB gzipped. Turn on **Settings → Public CDN**: the dashboard shows this snippet prefilled for your project, with a live preview.

```html
<script src="https://app.glotlang.com/widget/locale-switcher.global.js"
        data-project="your-slug" data-cdn="https://app.glotlang.com" defer></script>

<glot-locale-switcher variant="menu" strategy="query:lang"></glot-locale-switcher>
```

It renders a compact button with the current language (🇪🇸 ES); a click opens the list of every language, each with its flag, its own name and its code.

### The locales manifest

The element reads `GET /cdn/{slug}/locales.json?min=0.95`:

```json
{
  "source": "en",
  "locales": [
    { "code": "en", "name": "English", "dir": "ltr", "coverage": 1, "critical_coverage": 1, "ready": true, "version": 0 },
    { "code": "fr", "name": "Français", "dir": "ltr", "coverage": 0.98, "critical_coverage": 0.99, "ready": true, "version": 42 }
  ],
  "updated_at": "2026-11-07T10:00:00Z"
}
```

The source locale, then every target locale that is **ready**:

- `coverage`: catalog keys and the segments rendered by glot Edge pages together (non-stale translations / total) must be at least `min` (default `0`; the element asks for `0.95`);
- `critical_coverage`: the worst critical page (home, pricing, signup, checkout… set in Edge settings) — each must reach its threshold (98 % by default). "95 % of the keys" is not "95 % of the experience": one untranslated pricing page keeps the language out. It is `null` while glot Edge has not seen any page (key-only projects: only `coverage` counts);
- `version`: the locale's catalog version, the same one the CDN catalogs, the Edge translation memory and pre-rendered pages use.

Add `&all=1` to also list the locales that are not ready (`"ready": false`), e.g. for a dashboard. It is served with an ETag, `Cache-Control: public, max-age=60, stale-while-revalidate=600` and `Access-Control-Allow-Origin: *`. Add a language in glot and translate it: within about a minute of becoming ready it appears in every visitor's switcher.

### Attributes

| Attribute | Values | Default |
| --- | --- | --- |
| `strategy` | `query:NAME`, `prefix`, `prefix:always`, `cookie:NAME`, `event`, `auto` | `auto` |
| `variant` | `menu`, `pills`, `select` | `menu` |
| `labels` | `code` (FR / EN), `name` (Français / English): the trigger of `menu`, the pills, the select options | `code`, `name` for select |
| `flags` | `auto`, `off` (menu only) | `auto` |
| `flag-CODE` | an emoji or a region code, `""` for none: `flag-en="🇺🇸"`, `flag-en="us"`, `flag-ar="sa"`, `flag-fr=""` | |
| `search-label` | label of the menu's search field (shown above 8 languages) | `Search` |
| `min` | coverage threshold, 0 to 1 | `0.95` |
| `project`, `cdn` | override the script's `data-project` / `data-cdn` | |
| `aria-label` | label of the group / list | `Language` |

`menu` is now the default (it used to be `pills`). A page that sets `variant` keeps its variant; set `variant="pills"` to keep the row of pills on a page that did not.

### Flags

Languages are not countries, so a flag is only a hint. The menu uses:

- the region of the tag when there is one: `pt-BR` → 🇧🇷, `en-US` → 🇺🇸, `zh-TW` → 🇹🇼, `es-MX` → 🇲🇽. A continent (`es-419`) gets no flag;
- otherwise one representative country, only where it is the obvious default: `fr` → 🇫🇷, `de` → 🇩🇪, `ja` → 🇯🇵, `en` → 🇬🇧, `pt` → 🇵🇹, `zh` → 🇨🇳…;
- no flag for languages spoken in many countries or none (`ar`, `sw`, `ca`, `eo`, `la`…): the code in a small badge instead.

Override per language with `flag-CODE` attributes (lowercase code, read when the element renders) or the `flags` property, which re-renders:

```html
<glot-locale-switcher flag-en="🇺🇸" flag-ar="sa"></glot-locale-switcher>
<script>document.querySelector("glot-locale-switcher").flags = { en: "us", "pt-br": "🇧🇷" };</script>
```

An explicit region beats a language override (`flag-en="us"` leaves `en-GB` on 🇬🇧). `flags="off"` shows the code badges only. Windows does not draw flag emoji (it shows two letters): the element checks once and shows the code badge there.

### Keyboard and screen readers (menu)

The trigger is a button with `aria-haspopup="listbox"` and `aria-expanded`; the list is a `listbox` of `option`s with `aria-selected` on the current language, each with its own `lang` and `dir`.

- Enter, Space, Arrow Down or Arrow Up on the trigger opens the list; focus goes to the search field (above 8 languages) or the current language.
- Arrow Up / Down move, Home / End jump to the ends, typing a letter jumps to the next language starting with it.
- The search matches the language's own name, its code and its name in the visitor's language ("German" finds Deutsch); Enter picks the first match.
- Enter or Space picks; Escape closes and returns focus to the trigger; Tab or a click outside closes.
- The list opens on the inline-start side (right-aligned on RTL pages), flips up near the bottom of the window and does not animate with `prefers-reduced-motion`.

Strategies decide how the current locale is read and what a click does:

- `query:lang`: reads `?lang=`, navigates to the same URL with `?lang=fr`.
- `prefix`: reads the first path segment (`/fr/pricing`), navigates by swapping it. The source locale is unprefixed; use `prefix:always` if your source has a prefix too.
- `cookie:NAME`: reads the cookie, sets it (one year, `path=/`) and reloads.
- `event`: navigates nowhere. Sets `<html lang dir>`; your app swaps its strings.
- `auto`: `query:lang` when the URL has `?lang=`, `prefix` when the path starts with a locale, `event` otherwise.

Every strategy first dispatches a cancelable, bubbling `glot:locale-change` event (`detail: { locale, previous, dir }`); call `preventDefault()` to handle the change yourself. The choice is saved in `localStorage["glot-locale"]`. When nothing in the URL or cookie names a locale (or with `event`, which starts from the saved choice), the element uses `<html lang>`, then the saved choice, then the source.

Styling: shadow DOM with parts `trigger`, `flag`, `menu`, `search`, `option`, `option-current` (menu), `list`, `pill`, `pill-current` (pills), `select`, and custom properties `--glot-gap`, `--glot-radius`, `--glot-padding`, `--glot-font-size`, `--glot-fg`, `--glot-bg`, `--glot-border`, `--glot-list-bg`, `--glot-active-bg`, `--glot-active-fg`, `--glot-focus`, and for the menu `--glot-menu-bg`, `--glot-menu-fg` (default: the system `Canvas` / `CanvasText` pair), `--glot-menu-radius`, `--glot-menu-max-height`, `--glot-hover-bg`, `--glot-shadow`, `--glot-z`. The trigger inherits the page's color and font. Pills are buttons with `aria-current`; arrow keys (mirrored in RTL), Home and End move between them. Each label carries its own `lang` and `dir`.

```css
glot-locale-switcher { --glot-active-bg: #111; --glot-active-fg: #fff; --glot-border: #ddd; }
glot-locale-switcher::part(pill) { text-transform: lowercase; }
/* dark site: give the menu your surface colors */
.dark glot-locale-switcher { --glot-menu-bg: #18181b; --glot-menu-fg: #fafafa; }
glot-locale-switcher::part(flag) { font-size: 1.1em; }
```

### Strings over the air

The switcher only lists languages. Showing a new language without a redeploy also needs the app to **load its strings at runtime** instead of bundling them at build time. The same script exposes:

```js
const messages = await glot.loadCatalog("fr"); // GET /cdn/{slug}/fr.json, nested JSON
document.addEventListener("glot:catalog", (e) => console.log(e.detail.locale, e.detail.messages));
```

The browser's HTTP cache revalidates with the ETag after 60 s; there are no custom headers, so no CORS preflight.

What this means for your site:

- **Client-rendered apps** (SPA, Leptos/Yew CSR, Vue, React without SSR): load the catalog for the chosen locale with `glot.loadCatalog` and feed your i18n library. A new language then needs nothing on your side.
- **Catalogs compiled into the bundle** (`import fr from "./fr.json"`, `include_str!`, a Rust `i18n!` macro, static site generation): a language glot adds after the build is not in the bundle. The switcher can list it, but the strings come only from a runtime fetch. Keep the compiled catalogs as the fallback and fetch the others, or rebuild.
- **Server-rendered apps** (Next, Remix, Node): fetch on the server with `createOnDemandLoader` from `@glot-i18n/runtime/ondemand` (in-memory cache, ETag revalidation, deep merge over the source) and render real HTML.

### Leptos (CSR)

```rust
// index.html: the script tag above, and <glot-locale-switcher strategy="event"></glot-locale-switcher> in your header.
use wasm_bindgen::{closure::Closure, JsCast};

let (messages, set_messages) = signal(serde_json::Value::Null);
let load = move |locale: String| {
    let glot = js_sys::Reflect::get(&web_sys::window().unwrap(), &"glot".into()).unwrap();
    let f: js_sys::Function = js_sys::Reflect::get(&glot, &"loadCatalog".into()).unwrap().into();
    let p: js_sys::Promise = f.call1(&glot, &locale.into()).unwrap().into();
    leptos::task::spawn_local(async move {
        if let Ok(v) = wasm_bindgen_futures::JsFuture::from(p).await {
            set_messages.set(serde_wasm_bindgen::from_value(v).unwrap_or_default());
        }
    });
};
let on_change = Closure::<dyn Fn(web_sys::CustomEvent)>::new(move |e: web_sys::CustomEvent| {
    let locale = js_sys::Reflect::get(&e.detail(), &"locale".into()).unwrap().as_string().unwrap();
    load(locale);
});
document().add_event_listener_with_callback("glot:locale-change", on_change.as_ref().unchecked_ref()).unwrap();
on_change.forget();
```

A site that already switches with `?lang=xx` (and reloads) can keep doing so: use `strategy="query:lang"` and, at startup, `glot.loadCatalog(lang)` when `lang` isn't one of the compiled catalogs.

### Plain JS

```html
<glot-locale-switcher strategy="event" variant="select"></glot-locale-switcher>
<h1 data-i18n="home.title">Welcome</h1>
<script>
  const get = (o, k) => k.split(".").reduce((x, p) => x?.[p], o);
  document.addEventListener("glot:catalog", ({ detail }) => {
    for (const el of document.querySelectorAll("[data-i18n]")) {
      el.textContent = get(detail.messages, el.dataset.i18n) ?? el.textContent;
    }
  });
  document.addEventListener("glot:locale-change", (e) => glot.loadCatalog(e.detail.locale));
  // The script is deferred: `glot` exists from DOMContentLoaded on.
  addEventListener("DOMContentLoaded", () => {
    const saved = localStorage.getItem("glot-locale");
    if (saved) glot.loadCatalog(saved);
  });
</script>
```
