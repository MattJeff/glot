# Is a language live on your site?

A language can be 100 % translated in glot and still not show on your site. glot translates and publishes; your site's own code decides whether it accepts the language. The usual cause is the language **code**: glot calls Simplified Chinese `zh-CN`, your site only accepts two- or three-letter codes such as `zh`, so `?lang=zh-CN` is ignored and the page stays in its default language.

glot handles this in two ways: it accepts the codes your site uses, and it checks every language on your real site and tells you what to change.

## Codes glot accepts

Every public endpoint that takes a locale resolves the code against your project's languages, exact match first:

| Your site asks for | Your project has | glot serves |
| --- | --- | --- |
| `zh`, `zh-cn`, `zh_CN`, `zh-Hans`, `zh-Hans-CN` | `zh-CN` | `zh-CN` |
| `zh` | `zh-CN` and `zh-TW` | `zh-CN` (the usual default for a bare `zh`) |
| `zh-HK` | `zh-CN` and `zh-TW` | `zh-TW` (same script) |
| `pt-BR` | `pt` | `pt` |
| `pt` | `pt-PT` only | `pt-PT` |
| `en-GB` | `en-US` | `en-US` |
| `zh-TW` | `zh-CN` only | 404: Traditional Chinese is never served Simplified |

So `GET /cdn/{slug}/zh.json` returns your `zh-CN` catalog. The same holds for the Edge translation memory (`/edge/v1/{slug}/{locale}/tm`). A code that resolves this way never starts an [on-demand translation](/docs/runtime).

## The code your site uses

When your site uses another code than glot for a language, set it in **Settings → Locales → Live on your site → Code on your site** (or `PUT /api/projects/{slug}/site-codes` with `{"zh-CN": "zh"}`).

- `/cdn/{slug}/locales.json` then lists the language with `"site_code": "zh"` next to `"code": "zh-CN"`. The field is absent when both are the same.
- [`<glot-locale-switcher>`](/docs/runtime#language-switcher-live-languages) writes the site code wherever your site reads it: `?lang=zh` (`query:`), `/zh/…` (`prefix`), the cookie value (`cookie:`), and `detail.locale` of the `glot:locale-change` event (`detail.glotLocale` keeps glot's code). It recognizes both codes as the current language.

## How glot checks

**Your visitors.** After the switcher sends a visitor to a language, the next page load compares what was asked with what the page shows: `<html lang>`, and whether the visible text changed. It sends one small report per language and result, per browser, per day: the language, the `<html lang>` seen, applied or not, and the page's origin. No URL path or query, no cookie, no identifier. Reports are capped per day and, once your site URL is set, accepted from that site only.

**A probe from glot's servers.** When you ask (**Check again**), and after each translation job, glot fetches your home page for the language, using the site URL and URL pattern of **Settings → Your site** (`{route}?lang={locale}` or `/{locale}{route}`). It compares it with two pages: the one your site answers for a code it cannot know (`xx`), and your source-language page. It looks at the `<html lang>`, the title and the share of visible text that differs. Both comparisons are needed: some sites keep their default language for a code they reject, but fall back to English for an unknown short code. If the page is the same, it tries the usual spellings (`zh`, `zh-cn`, `zh_CN`, `zh-Hans`).

- The page changes with glot's code: **live**.
- It only changes with another spelling: your site **does not display** the language, and that spelling is its code. When that page also declares the language in `<html lang>`, glot saves it as the site code by itself and says so; otherwise it proposes it.
- Same HTML for every code: many sites switch language in the browser, so this proves nothing. The state stays **not confirmed** until a visitor picks the language.

Visitor reports outrank the probe.

## "Translated, but your site does not display it"

Three ways out, shown next to the language:

1. **Use `zh` on my site.** One click: glot sends your site the code it accepts. Nothing changes in your code. This button appears only when glot saw your site show the language under that code. glot never guesses: a site may accept `zh` and still fall back to another language, because it has no Chinese strings at all.
2. **Accept the full code in your site.** Wherever your code reads the language, accept a region or script too:

   ```js
   // Before: only "zh" passes
   if (/^[a-z]{2,3}$/.test(lang)) setLanguage(lang);

   // After: "zh-CN", "pt-BR", "zh-Hant-TW" pass too
   if (/^[a-z]{2,3}([-_][A-Za-z0-9]{2,8})*$/.test(lang)) setLanguage(lang);
   ```

   Then load the catalog under that same code: `glot.loadCatalog(lang)` or `GET /cdn/{slug}/{lang}.json`.
3. **Send this to my developer.** A ready-to-copy message that describes the problem and both fixes.

## Limits

- The probe reads the home page's HTML only, without running JavaScript. Sites rendered in the browser are settled by visitor reports, which need `<glot-locale-switcher>` with a `query:`, `prefix` or `cookie:` strategy. With `event`, your app swaps the strings itself and nothing is reported.
- A site that never updates `<html lang>` is judged on its text: if the text changed after the switch, the language counts as applied.
- A site on a private address (`localhost`) cannot be probed from glot's servers.
