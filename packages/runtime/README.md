# @glot/runtime

Long-tail locales at zero cost: when the visitor's language isn't shipped, the UI is translated **on the
device** (Chrome Translator API) with glot's guarantees — placeholders/tags/glossary masked, plural
structure kept, every string validated by glot-core (WASM). Anything that fails stays in the source
language; nothing unvalidated is ever shown.

```sh
npm i @glot/runtime
```

Serve `node_modules/@glot/runtime/dist/glot.wasm` somewhere (e.g. `public/glot.wasm`) and pass `wasm="/glot.wasm"`,
or let your bundler resolve it next to the module.

## Plain JS

```ts
import { fallbackWithBadge } from "@glot/runtime";

await fallbackWithBadge({ source: en, glossary, locale: "de", onMessages: (m) => render(m) });
```

`createFallbackMessages` is the same without the badge; it never throws and returns a `status`
(`translated`, `unsupported`, `needs-activation`, …).

## next-intl (App Router) — Client **and** Server Components

```tsx
// app/layout.tsx (Server Component)
import { NextIntlClientProvider } from "next-intl";
import { getMessages } from "next-intl/server";
import { GlotFallbackProvider } from "@glot/runtime/next-intl";

<NextIntlClientProvider>
  <GlotFallbackProvider shipped={["en", "fr"]} source={await getMessages()} glossary={glossary} wasm="/glot.wasm">
    {children}
  </GlotFallbackProvider>
</NextIntlClientProvider>
```

- **Client Components** (`useTranslations`): nothing to change, they re-render with the translation.
- **Server Components**: replace `getTranslations` with `getGlotTranslations`:

  ```tsx
  import { getGlotTranslations } from "@glot/runtime/next-intl/server";

  const t = await getGlotTranslations("home");
  return <h1>{t("title", { name })}</h1>;
  ```

  `t()` returns a tiny client element (`<T k="home.title" values={…}>server text</T>`). The server HTML is
  still the shipped text (SEO, no-JS); hydration renders the same text, then — only when the browser's
  language isn't shipped — the element swaps to the validated on-device translation. No DOM patching, so
  React never sees a mismatch, and RSC navigations keep working. You can also use `<T k values>` directly.

  Limits: `t()` returns elements, so keep next-intl's `t` for attributes (`alt`, `placeholder`),
  `generateMetadata` and `t.rich`; those stay in the shipped language. For real server-rendered pages in
  unshipped locales (SEO), see **On-demand locales** below.

Example + end-to-end test (real Next build, fake Translator, `de` not shipped): `examples/runtime-next`
(`pnpm install && pnpm test`).

## On-demand locales (server-rendered, SEO)

On-device translation only happens in the browser. To get a **server-rendered** page in a locale you don't ship,
let the glot server translate it the first time someone asks:

```ts
// i18n/request.ts
import { getRequestConfig } from "next-intl/server";
import { headers } from "next/headers";
import { createOnDemandLoader, acceptedLanguages } from "@glot/runtime/ondemand";
import en from "../messages/en.json";

const ondemand = createOnDemandLoader({ server: "https://glot.example.com", project: "my-app", source: en });

export default getRequestConfig(async () => {
  const lang = acceptedLanguages((await headers()).get("accept-language"))[0]?.split("-")[0];
  const messages = lang && lang !== "en" ? await ondemand(lang) : null;
  return messages ? { locale: lang!, messages } : { locale: "en", messages: en };
});
```

and in the layout pass `shipped={["en", await getLocale()]}` to `GlotFallbackProvider` (a locale glot already
serves needs no on-device pass) and `<html lang={await getLocale()}>`.

- First visit in `de`: the loader calls `GET /cdn/my-app/de.json?ondemand=1`, glot answers **202** and queues a
  normal translate job (glossary, validation, budget); the page renders in English and the browser's on-device
  fallback translates it. Once the job is done, every server render is German HTML.
- Server side, enable it per project (public CDN required): `PUT /api/projects/my-app/ondemand`
  `{"locales": ["de", "it"], "monthly_budget_usd": 5, "per_day": 5}` (`["*"]` = any locale). Off by default;
  403 / 402 / 429 when not allowed, over the monthly budget or over the daily cap — the loader just returns `null`.
- The loader keeps catalogs in memory, dedupes concurrent requests and revalidates in the background with the ETag
  (`ttl`, default 60 s). glot unreachable → last catalog, else `null`. No dependencies: Node, Edge, Workers.
- Locale URLs (`/de/…`) and static sites: `packages/edge` — `glotProxy` for Next 16 `proxy.ts` (rewrite + `x-glot-locale`
  header, cookie / Accept-Language redirect) and a Cloudflare Worker that fills `data-glot="key"` elements of any HTML page.

End-to-end test (real glot-server with the mock LLM on :8797 + Next): `examples/runtime-next/test/ondemand.test.ts`.

## i18next

```ts
import { GlotFallbackBackend } from "@glot/runtime/i18next";
// chain after your normal backend; use `react: { bindI18nStore: "added" }` to re-render after a click-to-download
```

## Development

```sh
pnpm build:wasm && pnpm build && pnpm test
```
