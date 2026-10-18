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
