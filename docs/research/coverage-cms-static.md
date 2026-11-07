# Couverture glot — générateurs statiques et CMS (« cms-static »)

> Relevé du 2026-11-07. Popularité : npm (semaine du 2026-10-31 au 2026-11-06), GitHub (étoiles), PyPI (30 jours),
> RubyGems (total), W3Techs (part des sites). « Avant » = binaire `target/integ/debug/glot` avant ce lot, testé sur
> des fixtures dans `scratchpad/cms-static/` (`init --yes --provider mock`, `translate`, `check`) ; « après » = même
> parcours avec ce lot.

## Matrice

| Framework / format | Popularité (source, 2026-11-07) | Format de catalogue | Statut avant | Statut après | Reste à faire |
|---|---|---|---|---|---|
| WordPress thèmes / plugins (POT/PO/MO + JSON JS) | 39,4 % des sites, 58,2 % des CMS (W3Techs) ; `@wordpress/i18n` 1 062 201 dl/sem. | `languages/{domain}-{locale}.po` (thèmes `languages/{locale}.po`), source `{domain}.pot`, locales `fr_FR`, printf `%s`/`%1$s`, `msgid_plural` | non détecté (stack `generic`, `locales/{locale}.json`) ; le `.pot` n'était jamais lu comme source ; nouvelle entrée cible : `msgid_plural` = singulier, sans références ni `php-format` | **géré via fichiers** : détection (en-tête `Plugin Name:`/`Theme Name:`, `Text Domain`, `Domain Path`), `.pot` = source, entrées nouvelles copiées du `.pot` (références, drapeaux, `msgid_plural`), validation printf + pluriels | `.mo` / `.l10n.php` / JSON JS restent à générer par WP-CLI (`wp i18n make-mo`, `make-php`, `make-json`) ; extraction PHP `__()`/`_n()` absente (`wp i18n make-pot`) |
| Shopify thèmes | 5,3 % des sites, 7,9 % des CMS (W3Techs) ; Shopify/dawn 3 107 ★ | `locales/en.default.json` (source) + `locales/fr.json`, `*.schema.json` (éditeur de thème), Liquid `{{ var }}`, pluriels `one`/`other` en sous-objet, bannière `/* */` | stack `generic`, source prise pour `fr` (le `.default` ignoré), `.schema.json` absent, fichier avec bannière illisible | **géré via fichiers** : détection (`layout/theme.liquid`), source `.default`, fichiers schema, bannière conservée, syntaxe `template` (`{{ }}` exigés à l'identique, pluriels complétés par CLDR) | Clés présentes à la fois dans le storefront et le schema : lues une fois |
| Hugo (`i18n/*.toml\|yaml\|json`) | gohugoio/hugo 90 407 ★ | `i18n/{locale}.toml` : `id = "…"` ou `[id] one/other`, go-i18n `description`, `{{ .Count }}` / `{{ . }}` | absent : TOML non lu, YAML/JSON non détectés | **géré via fichiers** : nouveau format TOML (`formats/toml_i18n.rs`, patch via `toml_edit` : commentaires et ordre conservés), détection (config + `[languages.*]`), syntaxe `template`, catégories CLDR par locale (pl : `few`/`many`) ; YAML : pluriels en sous-objets aussi dans un nouveau fichier | Menus et `languages.*.title` de la config non lus ; table imbriquée vidée laissée vide |
| go-i18n v2 (`active.*.toml`) | (même format que Hugo) | idem, `hash` conservé | absent | **lu/écrit** par le même format (détection laissée au périmètre back-end) | Détection `glot init` |
| Jekyll + jekyll-polyglot | jekyll 51 749 ★, gem 44 804 823 ; jekyll-polyglot 270 386 dl, 498 ★ | `_data/{lang}/*.yml` (`site.data[site.active_lang]`), `languages:` dans `_config.yml` | non détecté | **géré via fichiers** : détection, syntaxe `template`, pluriels en sous-objets | Pages : `[[docs]]` manuel (pas de preset, emplacement libre) |
| Markdown/MDX entiers (Docusaurus, VitePress, Starlight, MkDocs, Hugo content, Jekyll, Eleventy, Gatsby MDX) | astro 11 526 912 dl/sem. ; @astrojs/starlight 2 164 640 ; vitepress 1 970 529 ; @docusaurus/core 1 959 914 ; mkdocs 13 132 209 dl/mois, mkdocs-material 11 666 824, mkdocs-static-i18n 339 973 | pages par dossier ou suffixe de locale, front matter YAML/TOML | **absent** (aucune traduction de document) | **natif** : `glot docs` (segmentation `crates/core/src/markdown.rs`, code/liens/balises/JSX/shortcodes masqués en `{n}` et vérifiés par le validateur, front matter `title`/`description`/…, `lang:` réécrit), presets `[[docs]]` écrits par `glot init`, traductions dans `.glot-docs/` (catalogues + lock propres, `glot check` y fonctionne), relecture des retouches manuelles, pages restructurées à la main protégées (`--force`) | Attributs JSX (`<Card title>`), `nav_translations` MkDocs, `themeConfig` VitePress ; paragraphes traduits réécrits sur une ligne ; HTML en bloc multi-lignes traité comme texte |
| Docusaurus `code.json` / thème | (voir ci-dessus) | `i18n/{locale}/code.json` + `docusaurus-theme-classic/*.json`, `{message, description}` | détecté par `init_web` (autre périmètre) mais `format = "auto"` : `description` traduite si le fichier source n'existe pas encore | `format = "json-message"` forcé pour la stack `docusaurus` | `usePluralForm` (`a\|b`) vérifié comme texte ; source à générer par `write-translations --locale en` |
| Astro Starlight UI | (voir ci-dessus) | `src/content/i18n/{locale}.json`, clés plates pointées | détecté (`init_web`) ; nouvelle cible écrite imbriquée (`{"search": {"label"}}`) | `[i18next] key_separator = false` écrit par `init` : cibles plates | — |
| Gatsby (gatsby-plugin-react-i18next) | gatsby 288 021 dl/sem. ; plugin 7 813 | i18next `locales/{lng}/{ns}.json` | **OK** (stack i18next) | inchangé ; pages MDX via `glot docs` | — |
| Eleventy (i18n plugin) | @11ty/eleventy 279 777 dl/sem. ; eleventy-plugin-i18n 1 627 | pas de convention ; plugin tiers : un fichier `{clé: {en, fr}}` | non détecté | **documenté** : `files` vers `_data` par locale, pages via `[[docs]]` | Format `{clé: {en, fr}}` (toutes locales dans un fichier) non lu |
| Webflow | 0,8 % des sites (W3Techs) | Localization : variantes d'item par `cmsLocaleId`, Pages API | — | **documenté** (export JSON par locale, réimport API) | Connecteur |
| Framer | 0,2 % des sites (W3Techs) | pas d'API ; plugins XLIFF/CSV | — | **documenté** (XLIFF lu par glot) | Connecteur |
| Contentful | contentful 1 665 484 dl/sem. | champs localisés `fields.x["en-US"]` | — | **documenté** (export/import CLI, JSON plat par locale) | Connecteur |
| Sanity | sanity 1 529 863 dl/sem. | document-level ou field-level (internationalized-array) | — | **documenté** (export GROQ, mutation) | Connecteur |
| Strapi 5 | @strapi/strapi 271 252 dl/sem. | document-level (`documentId` + `locale`) | — | **documenté** (REST `?locale=`) | Connecteur |

## Ce qui a été ajouté

- `Syntax::Template` (`syntax = "template"`) : `{{ … }}` (Liquid, Go templates) comparés à l'identique, apostrophes
  neutres, pluriels en sous-objets complétés par CLDR ; message d'erreur `{{ .Name }}`.
- Format TOML (`crates/core/src/formats/toml_i18n.rs`, `Format::Toml`, extension `.toml` sous `auto`) via `toml_edit`
  (déjà dans l'arbre par `toml` 0.8 ; évite un parseur maison pour chaînes multi-lignes et échappements).
- Config : source Shopify `{locale}.default.json`, source WordPress `.pot` (plugin et thème).
- JSON : bannière `/* … */` lue et réécrite telle quelle.
- PO : nouvelles entrées copiées du `.pot` voisin.
- YAML : pluriels en sous-objets dans un nouveau fichier Hugo `i18n/` / Jekyll `_data/`.
- `glot init` : `crates/cli/src/init_cms.rs` (Shopify, WordPress, Hugo, Jekyll) + presets `[[docs]]`.
- `glot docs` : `crates/cli/src/docs.rs` + `crates/core/src/markdown.rs`.

## Vérifié

- Fixtures (scratchpad) : Hugo TOML et YAML (fr, pl), Shopify (fr + schema), plugin WordPress (fr_FR, de_DE),
  Jekyll polyglot, Docusaurus (docs + `code.json`), Starlight (MDX + UI), Hugo content (shortcodes).
- `glot check` attrape `{{ .Nom }}` au lieu de `{{ .Name }}` et `few` manquant en pl.
- `glot docs` : 2e passage = 0 page réécrite ; retouche manuelle relue (non écrasée) ; paragraphe source modifié =
  1 segment retraduit ; page restructurée à la main = ignorée avec avertissement.
