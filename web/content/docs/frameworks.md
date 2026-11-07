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
| iOS / macOS (SwiftUI, UIKit) | Foundation (`.xcstrings`, `.strings`) | Only catalog keys; code untouched unless `--rewrite` |

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

## iOS / macOS (Swift)

`glot init` on an Xcode project sets `[extract] include = ["**/*.swift"]` (tests excluded). No Swift toolchain is needed: glot reads Swift literals itself (`"…"`, `"""…"""`, `#"…"#`, `\(…)`).

- `glot extract` lists the keys the code looks up: SwiftUI text APIs (`Text`, `Button`, `Label`, `Toggle`, `Picker`, `TextField`, `Section`, `Menu`, `.navigationTitle`, `.alert`, `.confirmationDialog`, `.help`, `.accessibilityLabel`, `ContentUnavailableView`…), `NSLocalizedString`, `String(localized:)`, `LocalizedStringKey`/`Resource` and SwiftGen-style `L10n.tr`. Plain `String`s shown by UIKit (`label.text = "…"`, `setTitle(_:for:)`, `UIAlertController`) are listed as hardcoded. Logs, image and SF Symbol names, `Text(verbatim:)`, previews and tests are ignored.
- Keys follow SwiftUI: `Text("\(n) countries")` looks up `%lld countries`. The specifier comes from the interpolated value's type (`Int` → `%lld`, `Double` → `%lf`, `String` and objects → `%@`, `specifier:` wins), found in the project's declarations. An unknown type falls back to `%@` with a warning: check it against Xcode's *Export Localizations*.
- `glot i18nize` adds the missing keys to the source catalog, in their table (`tableName:`, SwiftGen table, else `Localizable`). SwiftUI code is already localizable, so it is not rewritten. `--rewrite` wraps the UIKit strings in `String(localized: "…")`; the file is re-read before being written.
- `glot check --fix-suspicious` deletes keys copied from code by a broken script (`— \(L10n.tr(`, `\u{2192}`) when no code looks them up.

## Mobile and desktop catalogs

For these stacks glot works on the files you already have: `glot init` finds them, `glot translate` patches them in place (untouched entries, comments, attributes and order are kept byte for byte; in JSON and YAML a new key goes where the source file has it, and a new JSON file copies the source's layout: one line, or its indentation), and `glot check` validates placeholders and plurals in the file's own syntax. No runtime or code change is needed. "Native" means `i18nize`/`extract` also understand the code; "files" means catalogs only.

| Stack | Status | Files `glot init` detects | Syntax checked | Limits |
|---|---|---|---|---|
| iOS / macOS String Catalog | native | `**/*.xcstrings` (locales from the file) | `%@ %lld %1$@`, plural / device variations | Substitutions (`%#@x@`) are kept but checked as text |
| iOS / macOS `.strings` + `.stringsdict` | native | `<dir>/{locale}.lproj/*.strings`, `*.stringsdict` (`Localizable` first, `InfoPlist` last) | printf, `NSStringPluralRuleType` plurals | `English.lproj`-style names must be renamed to ISO codes; UTF-16 kept as is |
| Android | files | `res/values-{locale}/strings.xml` (`values` = source, `pt-rBR`, `b+sr+Latn`) | `%1$s %d`, `<plurals>`, `<string-array>` (`name[0]`), `translatable="false"` skipped | Other files (`arrays.xml`) need a second pattern in `files` |
| Compose Multiplatform | files | `composeResources/values-{locale}/strings.xml` | as Android | — |
| moko-resources (KMP) | files | `moko-resources/{locale}/strings.xml` + `plurals.xml` (`base` = source; older `MR/base`) | as Android, `<plural>` | — (a new plural key goes to `plurals.xml`, created if needed) |
| Flutter gen-l10n (`intl`) | files | `l10n.yaml` `arb-dir` / `template-arb-file`, else `lib/l10n/app_{locale}.arb` | ICU (`{name}`, `plural`, `select`), `@key.description` as note | — |
| Flutter easy_localization | files | `assets/translations/{locale}.json` (also `assets/i18n`, `assets/lang`, `assets/l10n`) | `{}` / `{name}`, `zero/one/other` sub-objects as plurals | `@:key` links are text |
| Flutter slang | files | `lib/i18n/strings.i18n.json` (base, `base_locale` from `slang.yaml`/`build.yaml`, `en` by default) + `strings_{locale}.i18n.json`; same with `.i18n.yaml`; CSV per locale (`strings_{locale}.i18n.csv`, `key,value` rows) or compact (`strings.i18n.csv` with a `key,en,de-DE` header, every locale in one file: a new locale gets a column) | `$name`, `${name}`, plural sub-objects / `apple.one`…`apple.other` CSV rows | `@:key` links are text |
| React Native / Expo + i18next | files | `package.json` with `react-native`/`expo` + i18next → its JSON (`locales/{locale}/translation.json`…) | i18next `{{x}}`, `_one/_other` | Same as i18next on the web |
| React Native / Expo + i18n-js | files | `locales/{locale}.json` (`i18n-js` in `package.json`) | `%{name}`, `zero/one/other` sub-objects | — |
| Lingui (RN or web) | native | `src/locales/{locale}/messages.po` | ICU | — |
| Electron + i18next | files | i18next JSON (`locales/{locale}/translation.json`) | i18next | — |
| .NET (MAUI, WPF, WinForms, ASP.NET) | files | `Strings.resx` (source) + `Strings.{culture}.resx` | `{0}`, `{0:N2}`, `{{` | — |
| Qt (Widgets, QML) | files | `<dir>/<app>_{locale}.ts` (XML, `format = "qt"`); source text comes from the files (`sourcelanguage`, `en` by default) | `%1`, `%L1`, `%n`, `<numerusform>` plurals; `comment` disambiguates, `extracomment` / `location` become notes; only the numerus forms Qt stores for the language are required (fr: 2) | Run `lupdate` first (glot does not scan C++/QML); `type="unfinished"` with text is treated as needing review, and is cleared when glot writes a translation |
| Unity Localization | files | `ProjectSettings/` + the String Table CSV exports under `Assets/` (`Key,Id,Shared Comments,English(en),French(fr)…`), every locale in one file | Smart String `{0}`, `{name}`; `Shared Comments` become notes | Export each table with the package's CSV extension first and import it back after (CSV or CSV (Merge)); a new key gets `Id` `0` (Unity assigns one), a new locale a `German(de)` column (add the Locale in Unity too). `.asset` files are not read |

```sh
glot init --yes     # detects the stack and writes glot.toml
glot translate      # patches the target files
glot check          # placeholders, plurals, length
```

## JavaScript web libraries

`glot init --yes` reads `package.json` and the library's config, writes `files`, `syntax`, the locales your config declares (Transloco `langs`, Nuxt / Astro / Docusaurus / Lingui / next-intl `locales`) and the source locale it names (Paraglide `baseLocale`, `defaultLocale`, Lingui `sourceLocale`, Transloco `defaultLang`, Angular `sourceLocale`; `--source` still wins), then `glot translate` patches the catalogs in place and `glot check` validates them in the library's syntax. "Native" means `i18nize` / `extract` also understand the code; "files" means glot works on the catalogs only.

| Library | Status | Files `glot init` detects | Syntax checked | Limits |
|---|---|---|---|---|
| i18next / react-i18next / next-i18next | native | the files imported by `src/i18n*`, else `public/locales/{locale}/{ns}.json` (namespaces = `ns:key`) | `{{x}}`, `$t(key)`, `<1>…</1>`, `_one/_few/_other` families completed per CLDR, `_male`/`_female` contexts | Arrays are translated item by item; numbers and booleans are kept, not copied to new targets |
| remix-i18next | native | as i18next, also `app/locales/{locale}.ts` dictionaries | i18next | — |
| next-intl | native | `messages/{locale}.json` | ICU (`plural`, `select`, `selectordinal`, `date`/`number` formats), rich-text tags | — |
| react-intl / FormatJS | native | `lang/{locale}.json`: flat `{id: message}` or `formatjs extract` objects (`defaultMessage` translated, `description` as a note) | ICU, `<b>…</b>` | Compiled catalogs (`--ast`) are build output: translate the extracted files, then `formatjs compile` |
| Lingui | native | `src/locales/{locale}/messages.po` (`lingui.config` path), or JSON (`minimal` and `lingui` styles: `translation` is written, `message`/`origin` kept) | ICU inside `msgstr`, `<0>…</0>` | Run `lingui extract` first; explicit ids and hashed ids are both keys |
| vue-i18n | files | `src/locales/{locale}.json` or `.yaml`, and the `<i18n>` blocks of single-file components as `src/{ns}.vue` (keys `components/Nav:home`; JSON, JSON5 or `lang="yaml"`, all locales in one block or one `<i18n locale="…">` block each) | `{name}`, `{0}`, `{'@'}`, `@:key` links, `a \| b \| c` plurals (same number of forms) | Only the locale's part of a block is rewritten (template, script, style and other locales keep their bytes); a missing locale is added to the multi-locale block, or as a new `<i18n locale>` block. `src="…"` blocks are read through their own file |
| Nuxt i18n | files | `i18n/locales/` (v9+, or `langDir`), `.json` / `.yaml`; locales and `defaultLocale` from `nuxt.config`; SFC `<i18n>` blocks as `components/{ns}.vue`, `pages/{ns}.vue` | as vue-i18n | Components with the same path under two top folders share a key prefix |
| Angular `@angular/localize` | files | `angular.json` `i18n.locales` → `src/locale/messages.{locale}.xlf` (XLIFF 1.2 and 2.0; the source is `messages.xlf`), or `messages.{locale}.xtb` (XTB, the source is the `messages.xmb` of `--format xmb`) | `<x id="INTERPOLATION"/>`, `<ph>`, `<pc>` kept identical, ICU plurals in `<source>`; XMB/XTB `<ph name="…"/>` as `{INTERPOLATION}` (`desc`/`meaning` are notes) | Run `ng extract-i18n` first. In XTB, a literal all-caps `{WORD}` in a message is written back as a placeholder |
| ngx-translate | files | `public/i18n/{locale}.json` (Angular 17+) or `src/assets/i18n/` | `{{ name }}` | — |
| Transloco | files | `rootTranslationsPath` (default `src/assets/i18n/{locale}.json`), scopes `…/{scope}/{locale}.json` as `scope:key`; `langs` from `transloco.config` | `{{ name }}` | — |
| svelte-i18n | files | `src/lib/i18n/locales/{locale}.json` (also `src/locales`, `src/lang`) | ICU | — |
| sveltekit-i18n | files | `src/lib/translations/{locale}/*.json` | `{{name}}` | Modifiers (`{{count; 0:none; default:many}}`) are checked as a placeholder only |
| Paraglide JS (inlang) | files | `project.inlang/settings.json`: `pathPattern`, `locales` | ICU-style `{name}`; variants with one plural selector become a plural (the target gets its CLDR categories, `declarations`/`selectors` are copied from the base locale, never translated) | Several selectors: one entry per match (`key#gender=female`), no new categories |
| Astro i18n routing | files | the UI dictionary of the Astro recipe (`src/i18n/ui.ts`: `{ en: {…}, es: {…} }`), locales from `astro.config` | ICU | Pages are content, not catalogs |
| Astro Starlight | files | `src/content/i18n/{locale}.json` (UI strings) | i18next | Docs pages (`src/content/docs/{locale}/`) are Markdown: [`glot docs`](#markdown-and-mdx-pages) |
| Docusaurus | files | `i18n/{locale}/code.json` and the theme/plugin JSON (`docusaurus-theme-classic/navbar.json`, `footer.json`, `docusaurus-plugin-content-docs/current.json`…) | `{name}` interpolation (ICU) | `usePluralForm` messages (`One post\|{count} posts`) are checked as plurals (one form per CLDR category of the language). Generate the source first: `npm run write-translations -- --locale en`, then `--locale fr` for each target. Markdown docs and blog posts: [`glot docs`](#markdown-and-mdx-pages) |
| VitePress | pages | locales are folders of Markdown pages (`docs/fr/`) | — | Translated with [`glot docs`](#markdown-and-mdx-pages); UI labels live in `.vitepress/config` (not read) |
| ember-intl | files | `translations/{locale}.yaml` / `.json` (`en-us`) | ICU (plurals stay one string) | — |
| qwik-speak | files | `i18n/{locale}/{asset}.json` (`en-US`) | `{{name}}` | — |
| @solid-primitives/i18n | files | `src/i18n/{locale}.ts` dictionaries (function values are kept, not translated) | `{{ name }}` (`resolveTemplate`) | — |

```sh
glot init --yes              # detects the library, writes glot.toml
glot translate               # patches the target catalogs
glot check                   # placeholders, plurals, tags in the library's syntax
```

## Static sites and CMS themes

`glot init --yes` recognizes these projects from their own files, then `glot translate` patches the catalogs in place (comments, order and untouched values kept) and `glot check` validates them in the template's syntax. With `syntax = "template"`, every `{{ … }}` expression (Liquid `{{ count }}`, `{{ price | money }}`, Go `{{ .Count }}`) must come back unchanged, and `one`/`other` plural tables get the target language's CLDR categories (`few`, `many` for Polish…).

| Stack | Status | Files `glot init` detects | Syntax checked | Limits |
|---|---|---|---|---|
| WordPress plugin | files | root PHP file with `Plugin Name:`; `Text Domain` + `Domain Path` → `languages/{domain}-{locale}.po`, the `.pot` as source (`en`), targets as WordPress locales (`fr_FR`) | printf (`%s`, `%1$s`), `msgid_plural` | New target entries copy the `.pot` block (references, `php-format`, `msgid_plural`). WordPress loads compiled files: run `glot compile` once — it writes each `.mo` and, for the strings referenced from `.js` files, the `{domain}-{locale}-{md5}.json` files `wp_set_script_translations` loads (as `wp i18n make-json`; needs the `X-Domain` header `wp i18n make-pot` writes). Later `glot translate` runs keep them in sync. `.l10n.php` (WP 6.5+) still comes from `wp i18n make-php` (WordPress falls back to the `.mo` without it) |
| WordPress theme | files | `style.css` with `Theme Name:` → `languages/{locale}.po` + the folder's `.pot` | as above | as above |
| Shopify theme | files | `layout/theme.liquid` → `locales/{locale}.json` (source `en.default.json`) and `locales/{locale}.schema.json` (theme editor) | Liquid `{{ name }}`, `one`/`other` sub-objects, `_html` keys' tags | The `/* … */` banner is kept. A key present in both the storefront and the schema file is read once |
| Hugo | files | `hugo.toml` / `config.toml` (`baseURL`) → `i18n/{locale}.toml` (or `.yaml`, `.json`), locales from `[languages.*]` | Go templates `{{ .Count }}`, `{{ . }}`, `one`/`other` tables (in TOML a table with only `other` is one message, go-i18n `description` is a translator note) | The site `title` and menu `name`s are translated by `glot docs` into `[languages.{locale}]` (TOML config only; a `[languages.{locale}]` table must exist) |
| Jekyll + jekyll-polyglot | files | `_config.yml` `languages:` → `_data/{locale}/*.yml` (or `_data/{locale}.yml`) | Liquid `{{ }}` | Pages: [`glot docs`](#markdown-and-mdx-pages) with `lang:` in their front matter |
| Gatsby (gatsby-plugin-react-i18next) | files | i18next JSON `locales/{locale}/{ns}.json` | i18next | MDX pages: [`glot docs`](#markdown-and-mdx-pages) |
| Eleventy | not detected | no catalog convention | — | Point `files` at your `_data` JSON/YAML per locale; a single `{key: {en, fr}}` file (eleventy-plugin-i18n) is not read. Pages: `[[docs]]` with `source = "src/en/**/*.md"`, `target = "src/{locale}/{path}.{ext}"` |

```sh
glot init --yes     # e.g. stack: hugo, catalogs: i18n/{locale}.toml
glot translate      # patches i18n/fr.toml
glot check          # {{ .Count }} kept, plural categories complete
```

### Hosted CMS and site builders

These keep content in their own database; glot works on an export and you import the result. There is no built-in connector.

| Platform | Model | How to use glot |
|---|---|---|
| Webflow | Localization: each locale is a variant of the same CMS item (`cmsLocaleId`), static pages through the Pages API | Export the items of a locale as JSON (Data API) into `webflow/{locale}.json` (`{itemId: {field: text}}`), translate with `files = ["webflow/{locale}.json"]`, write back with `PATCH /collections/{id}/items/{itemId}?cmsLocaleId=…` |
| Framer | No translation API; marketplace plugins (Locale Sync, Framer Localization Sync) export XLIFF | Export one XLIFF per locale, point `files` at it (`.xlf` is read as XLIFF), translate, import with the same plugin |
| Contentful | Field-level: `fields.title["en-US"]` | Export with `contentful space export`, keep one JSON per locale (`{entryId.field: text}`), import with `contentful space import` or the CMA |
| Sanity | Document-level (`@sanity/document-internationalization`) or field-level (`internationalized-array`) | GROQ-export the source locale to `sanity/{locale}.json`, translate, write back with a mutation script |
| Strapi 5 | Document-level: one version per locale of the same `documentId` | `GET /api/{type}?locale=en` → `strapi/{locale}.json`, translate, `PUT /api/{type}/{documentId}?locale=fr` |

Keep the export flat (`id.field` keys) with rich text as Markdown or HTML: glot checks the tags and placeholders it holds.

## Markdown and MDX pages

`glot docs` translates whole pages: front matter values people read (`title`, `description`, `sidebar_label`, `summary`, `linkTitle`…), paragraphs, list items, headings, table cells, blockquotes and admonition titles. Fenced and indented code, math, HTML comments, MDX `import`/`export`, link targets, inline code, HTML/JSX tags, `{expressions}`, Hugo shortcodes and Liquid tags are kept byte for byte (the validator rejects a translation that drops one). A front matter `lang: en` becomes `lang: fr`.

`glot init` writes the `[[docs]]` entries for the site it finds:

| Site | `source` | `target` |
|---|---|---|
| Docusaurus | `docs/**/*.md*`, `blog/**/*.md*` | `i18n/{locale}/docusaurus-plugin-content-docs/current/{path}.{ext}`, `i18n/{locale}/docusaurus-plugin-content-blog/{path}.{ext}` |
| VitePress | `docs/**/*.md` (root locale) | `docs/{locale}/{path}.{ext}` |
| Astro Starlight | `src/content/docs/**/*.md*` (or `…/docs/en/**`) | `src/content/docs/{locale}/{path}.{ext}` |
| MkDocs + mkdocs-static-i18n | `docs/**/*.md` (suffix) / `docs/en/**/*.md` (folder) | `docs/{path}.{locale}.{ext}` / `docs/{locale}/{path}.{ext}` |
| Hugo | `content/**/*.md` (by file name) / `content/en/**/*.md` (by directory) | `content/{path}.{locale}.{ext}` / `content/{locale}/{path}.{ext}` |

`{path}` is the page path under the glob's folder without its extension, `{ext}` its extension. Pages already under a target locale (`docs/fr/…`, `intro.fr.md`) are never sources; add `exclude = ["docs/drafts/**"]` to skip more.

```sh
glot docs --dry-run   # pages, segments, estimated cost
glot docs             # translate, then write every page per locale
```

- Translations live in `.glot-docs/` (`{locale}.json` keyed `page#hash`, its own `glot.lock`; `cd .glot-docs && glot check` reviews them), not in your app's catalogs. Commit it.
- A paragraph that did not change is never retranslated; editing one retranslates only it. An untranslated segment keeps the source text, and a page whose content would not change is not rewritten.
- Hand edits in a translated page are read back as human translations (glot will not overwrite them). A page whose structure was changed by hand (paragraph added or removed) is skipped with a warning; `--force` regenerates it. Pages translated before glot are adopted when their structure matches the source.
- A paragraph wrapped over several lines in the source is wrapped the same way in the translation (at the source's longest line, never inside a link or code span, never starting a line with `-`, `#`, `1.`…).
- The readable attributes of HTML / JSX tags are translated: `title`, `alt`, `label`, `placeholder`, `aria-label`, `aria-description`, `description`, `caption`, `tooltip`, `summary` (`<Card title="…">`, `<TabItem label="…">`, `<img alt="…">`); other attributes, `{expressions}`, URLs and paths stay as they are.
- Site config labels are translated along with the pages (kept in `.glot-docs/` like the rest; a value you change by hand in the config is left alone):
  - MkDocs: the `nav:` titles go to `nav_translations` of each `- locale: fr` entry of the mkdocs-static-i18n `languages` (declare the language there first).
  - VitePress: the `themeConfig` texts of the root locale (`nav`/`sidebar` `text`, `outlineTitle`, `docFooter`, `footer`, the site `title`/`description`…) go to `.vitepress/locales/{locale}.json` (`"source text": "translation"`). Use it in the config:

    ```ts
    import fr from './locales/fr.json'
    const tr = (cfg, dict) => JSON.parse(JSON.stringify(cfg), (_, v) => (typeof v === 'string' && dict[v]) || v)
    // locales: { root: { label: 'English', lang: 'en' }, fr: { label: 'Français', lang: 'fr', ...tr({ title, description, themeConfig }, fr) } }
    ```
  - Hugo: the site `title` and the menus (`[[menus.main]]` `name`) go to `[languages.fr]` of `hugo.toml` (menus copied with their `pageRef`/`weight`, only `name` translated).
- Docusaurus plural messages in `code.json` (`"One doc tagged|{count} docs tagged"`, described as "Pluralized…") are checked as plurals: one form per CLDR category of the language, in the order zero, one, two, few, many, other (French needs three, Russian four).

## Back-end frameworks

Server frameworks keep their catalogs in files glot reads and writes as they are: `glot init --yes` finds them and sets the syntax, `glot translate` patches the target files in place (comments, order and untouched entries kept byte for byte, new keys appended, new files created next to the source), `glot check` validates each message in the framework's own placeholder and plural syntax. glot does not read server code (Ruby, Python, PHP, Java, C#, Go, Elixir): extract your strings with the framework's tool first, then translate.

| Framework | Status | Files `glot init` detects | Syntax checked (`syntax =`) | Limits |
|---|---|---|---|---|
| Ruby on Rails (I18n) | files | `config/locales/**/{locale}.yml` and `*.{locale}.yml` (`devise.en.yml`): one pattern per file, a key new to `fr` goes to the mirror of its source file | `%{name}`, `one`/`other` mappings completed with the target's CLDR categories (`rails`) | strftime formats (`date.formats`) are translated as text |
| Django | files | `locale/{locale}/LC_MESSAGES/django.po` (and `djangojs.po`); the msgids are the source | `%(name)s`, `%s`, `{name}` (`python-brace-format`), `msgid_plural` with the file's `Plural-Forms` (only the forms it declares are required: cs has no `many`), `msgctxt` (`printf`) | Run `makemessages -a` first; Django reads `.mo` files: `glot compile` writes them (or `compilemessages`), later writes keep them in sync |
| Flask-Babel / Babel | files | `<dir>/{locale}/LC_MESSAGES/messages.po`; without a source `.po`, `messages.pot` (in `<dir>` or a folder above, pybabel's default is the project root) is the source | as Django | Run `pybabel extract` + `pybabel update` first, `pybabel compile` after |
| Elixir / Phoenix (gettext) | files | `priv/gettext/{locale}/LC_MESSAGES/{domain}.po`, `priv/gettext/{domain}.pot` as source: new target entries copy its comments, references and `msgid_plural` | `%{name}`, `msgid_plural` (`printf`) | Run `mix gettext.extract` first (`--merge` is not needed); Phoenix compiles `.po` itself |
| PHP gettext (`gettext/gettext`, `ext-gettext`) | files | `locale/{locale}/LC_MESSAGES/{domain}.po` | `%s`, `%1$s`, `msgid_plural` (`printf`) | `ext-gettext` reads `.mo`: `glot compile` writes them, later writes keep them in sync |
| WordPress | files | see [Static sites and CMS themes](#static-sites-and-cms-themes) | `__()`, `_n()` strings: `%s`, `%1$s`, `msgid_plural` | `wp i18n make-pot` first, `glot compile` (`.mo` + script JSON) after |
| Laravel | files | `lang/{locale}.json` (text keys, new keys go there) + `lang/{locale}/*.php` (keys `file.key`); `resources/lang/` before Laravel 9; `composer.json` with `laravel/framework` | `:name` / `:Name` / `:NAME`, `a\|b` forms by the target's plural rule, ranges `{0}`, `[2,*]` kept (`laravel`) | PHP arrays are patched in place (quotes, comments, trailing commas kept) |
| Symfony | files | `translations/{domain}.{locale}.{yaml,xlf}` and `{domain}+intl-icu.{locale}.*` (`default_path` from `config/packages/translation.yaml`) | `+intl-icu` messages as ICU, the others `%name%`, `{{ value }}` (validators), `\|` forms with intervals `{0}`, `]1,Inf[` kept (`symfony`) | Only YAML and XLIFF translation files are supported (not `.php`, `.json`, `.csv`); a domain new to a target gets its own file |
| Spring / Spring Boot | files | `src/main/resources/messages_{locale}.properties` (`messages.properties` = source, English by default) with `pom.xml` / `build.gradle` | `{0}`, `{1,number}`, `{0,choice,…}`; a single `'` in a message with arguments is an error (`l'utilisateur {0}` prints `lutilisateur {0}`), write `''` (`java`) | ISO-8859-1 files stay ISO-8859-1 (`\uXXXX` for other characters), UTF-8 files stay UTF-8; a new `\`-continued value is written on one line |
| Java `ResourceBundle` | files | `<Bundle>_{locale}.properties` next to `<Bundle>.properties` | as Spring (`java`) | — |
| ASP.NET Core / .NET (`IStringLocalizer`, resx) | files | `Resources/<Name>.resx` (source) + `<Name>.{culture}.resx`, with a `*.csproj` | `{0}`, `{0:C2}`, `{{` literal (`printf`); `<comment>` becomes a translator note | `Form1.resx`-style designer files without culture siblings are ignored |
| Go (go-i18n v2) | files | `go.mod` with `nicksnyder/go-i18n` → `active.{locale}.toml` (also `.json`, `.yaml`, `{locale}.toml`; `translate.*.toml` work files skipped) | Go templates `{{.Name}}`, `one`/`other`… tables completed with the target's CLDR categories, `description` as a note, `hash` kept (`template`); in JSON / YAML too, `description` / `hash` next to the forms are kept and never translated | — |
| Go (`golang.org/x/text` gotext) | not supported | `locales/{locale}/out.gotext.json` | — | The `messages[]` catalog format is not read yet |
| Express / Fastify + i18next | files | i18next JSON (`locales/{locale}/translation.json`, i18next-fs-backend) | `{{x}}`, `_one/_other` (`i18next`) | Same as i18next on the web |
| Express + `i18n` (mashpie) | files | `locales/{locale}.json` | `%s`, `%d`, `one`/`other` sub-objects (`printf`) | Mustache `{{name}}` is kept as text, not checked |
| NestJS (nestjs-i18n) | files | `src/i18n/{locale}/*.json` (one pattern per namespace file) | `{name}`, `one`/`other`/`zero` sub-objects (`icu`) | `$t(ns.key)` references are checked as text |
| Strapi, Directus | not file-based | content lives in the database | — | Translate the content with [`glot db`](cli.md#glot-db) or an export (see [Hosted CMS](#hosted-cms-and-site-builders)); Strapi admin-panel strings are an inline object in `src/admin/app.ts` (not read) |

```sh
glot init --yes     # e.g. stack: spring, syntax: java, catalogs: src/main/resources/messages_{locale}.properties
glot translate      # patches messages_fr.properties
glot check          # {0} kept, apostrophes doubled
```

## Rust and WebAssembly (Leptos, Dioxus, Yew, Tauri)

`glot init --yes` reads every `Cargo.toml` in the repository. It picks the i18n crate in use, finds its catalogs and sets the source locale the app declares (`fallback_language`, leptos_i18n `default`, or the first column of a string table). `glot translate` patches the files in place and leaves unchanged entries byte for byte. `glot extract` reads `src/**/*.rs`: it lists the keys used by `t("key")`, `t!("key")`, `fl!(loader, "key")` and `t!(i18n, home.title)`, and the hardcoded copy in `view!`, `html!` and `rsx!` (text nodes, `placeholder` / `title` / `aria-label` / `alt`). `glot i18nize` rewrites Leptos `view!` copy for apps with their own `t(key)` string tables (see below); with the other crates, move that copy into the catalog yourself, or let your coding agent do it.

| Crate | Status | Files `glot init` detects | Syntax checked | Limits |
|---|---|---|---|---|
| Fluent: fluent-rs / fluent-templates / i18n-embed / dioxus-i18n | files | `locales/{locale}/*.ftl` (one pattern per file), `i18n/{locale}/{crate}.ftl` (i18n-embed `i18n.toml`), `locales/{locale}.ftl` | `{ $var }`, `{ -term }` and message references kept identical, `{ NUMBER($n) }`, select expressions (`*[other]` default required, CLDR categories per target locale) | Attributes are keys `id.attr`; changed values are re-indented with 4 spaces. A new target file follows the source file: same order, comments and `##` sections; a message added later goes after the one that precedes it in the source. A translated line that starts with `[`, `*` or `.` is not escaped |
| leptos_i18n | files | `[package.metadata.leptos-i18n]` `locales` / `locales-dir` / `namespaces` → `locales/{locale}.json` (or `.yaml`, `{locale}/{ns}.json`) | `{{ var }}`, `<b>…</b>`, `_one` / `_other` families completed per CLDR | `.json5` files are patched in place (comments, unquoted keys, trailing commas kept; a new file is written as plain JSON). Range plurals are not read; in `.json5`, `_one` / `_other` keys are translated one by one (a locale's extra categories such as `_few` are not added) |
| rust-i18n | files | the `i18n!("locales")` directory: `{locale}.yml` (`_version: 1`) or one `app.yml` / `app.toml` (`_version: 2`, every locale in the file: only the locale's leaves are patched, a new key gets its `[key]` table) | `%{name}` | JSON `_version: 2` files are read as plain JSON |
| gettext-rs | files | `po/{locale}.po`, `locale/{locale}/LC_MESSAGES/*.po` | printf, `msgid_plural` | — |
| Hand-rolled tables (`static DICT: &[(&str, &str, &str)]`) | files | every `.rs` file with a `("key", "fr", "en")` slice; the columns are named by a comment right above the table (`// (key, fr, en)`) or by the lookup code (`\|(_, fr, en)\|`) | `{name}` placeholders (`syntax = "plain"`: `{{x}}` and `<x>` are text) | A locale without a column (say `de` for `(key, fr, en)` tuples) is written to a flat JSON catalog per Rust file, `{crate}/locales/{locale}/{file}.json` (`web/locales/de/i18n.json`, sorted keys), and the Rust code is not touched: load it in the lookup as a fallback (snippet below). The same key in two tables (cliponit's `a1` in `landing.rs` and `tools.rs`) is one glot key: only one of the files gets it. New keys are appended to the first table; a later duplicate of a key is left as is; rows whose key is source text (`("Personnages", "People")`) and fixed-size arrays are not read. Inline pairs such as `tr("Bonjour", "Hello")` are reported by `glot extract` |
| Yew, Tauri | — | Yew has no standard i18n crate: one of the above is detected. A Tauri front end is detected through its JavaScript library (`package.json`) | — | — |

```sh
glot init --yes     # e.g. stack: rust (string tables), catalogs: web/src/i18n.rs, web/src/pages/landing.rs
glot translate      # fills the empty "en" cells of the tuples
glot check          # {n} kept
glot i18nize --dry-run   # view! copy -> {move || t("key")}, shown as a diff
```

A locale added to `target_locales` that the tuples have no column for goes to `{crate}/locales/{locale}/{file}.json`. Read it where the table is looked up, for example in `web/src/i18n.rs`:

```rust
use std::{collections::HashMap, sync::LazyLock};

static DE: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/de/i18n.json")).unwrap_or_default());

// in t(key), before the fr/en columns:
if locale == Locale::De {
    if let Some(v) = DE.get(key) { return v.clone(); }
}
```

`glot i18nize` (string tables with a `pub fn t(`) rewrites only what keeps the same type: a text node right between two tags (`<p>"Bienvenue"</p>` → `<p>{move || t("login_bienvenue")}</p>`) and a visible attribute of an HTML element (`placeholder="…"` → `placeholder=move || t("…")`). The call is `t` when the file imports it, else `crate::i18n::t` (also when a local variable is named `t`). The text becomes a new row of the first table; `glot translate` fills the other columns. Copy inside expressions (`if ok { "Envoyé" } else { … }`), component props, `html!` and `rsx!` is listed with the call to write by hand. On cliponit: 32 files rewritten, 104 keys added, 59 strings listed, and the crate still builds (`cargo check --target wasm32-unknown-unknown`).

## Other stacks

glot reads and writes catalogs for any library that uses JSON files (`format = "json-nested" | "json-flat"`) with ICU, i18next or MessageFormat 2 syntax. Without `i18nize` support for your framework, use `glot extract` to list hardcoded strings and your coding agent with the [MCP server](mcp.md) to convert them.
