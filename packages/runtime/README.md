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
  `generateMetadata` and `t.rich`; those stay in the shipped language. Server-side translation of unshipped
  locales (SEO for them) is out of scope here: ship the locale with `glot translate`.

Example + end-to-end test (real Next build, fake Translator, `de` not shipped): `examples/runtime-next`
(`pnpm install && pnpm test`).

## i18next

```ts
import { GlotFallbackBackend } from "@glot/runtime/i18next";
// chain after your normal backend; use `react: { bindI18nStore: "added" }` to re-render after a click-to-download
```

## Development

```sh
pnpm build:wasm && pnpm build && pnpm test
```
