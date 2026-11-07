# Couverture glot — bibliothèques i18n JS web (2026-11-07)

Périmètre : next-intl, i18next (react-i18next, next-i18next, remix-i18next), react-intl/FormatJS, Lingui, vue-i18n,
Nuxt i18n, Angular (`@angular/localize`, ngx-translate, Transloco), svelte-i18n, sveltekit-i18n, Paraglide/inlang,
Astro (i18n natif, Starlight), SolidJS, Qwik, Ember, Docusaurus, VitePress.

**Méthode.** 25 fixtures réalistes (`scratchpad/js-web/fx/`, hors repo), chacune passée par `glot init --yes --provider
mock`, `glot translate --provider mock --yes`, `glot check`, plus un `git diff` du résultat. Les erreurs ont ensuite été
injectées à la main dans les cibles (placeholder renommé, balise retirée, branche de pluriel perdue, lien `@:` supprimé)
pour vérifier que `check` les signale.

**Popularité.** Téléchargements npm de la semaine du 2026-10-31 au 2026-11-06 (`api.npmjs.org/downloads/point`) et
étoiles GitHub relevées le 2026-11-07 (`api.github.com/repos`). Les chiffres sont environ 2,5 fois ceux du relevé du
2026-10-18 (`i18n-methods.md`) pour tous les paquets, y compris `react` : l'écart vient du compteur npm, pas des usages.
Seul l'ordre relatif compte. Ils incluent la CI et les dépendances transitives.

## Matrice

Statut : **OK** = détecté par `init`, lu, réécrit avec un diff minimal, syntaxe validée ; **partiel** ; **absent**.

| Framework | dl/sem. npm | ★ GitHub | Format / emplacement | Avant | Après | Reste à faire |
|---|---:|---:|---|---|---|---|
| i18next + react-i18next | 34,8 M / 25,7 M | 8 662 (i18next) | JSON `public/locales/{lng}/{ns}.json` | OK (namespaces, pluriels v4, contextes, `$t`, `<1>`) | OK | Les nouvelles clés d'une cible sont écrites dans l'ordre alphabétique, pas dans l'ordre de la source |
| next-intl | 9,7 M | 4 393 | JSON imbriqué `messages/{locale}.json`, ICU | OK | OK | — |
| react-intl / FormatJS | 4,5 M (`@formatjs/cli` 1,2 M) | 14 753 (formatjs) | `lang/{locale}.json`, plat ou extrait `{defaultMessage, description}` | OK | OK | Catalogues compilés (`--ast`) : non gérés, ce sont des artefacts de build |
| vue-i18n | 4,9 M | 2 712 | `src/locales/{locale}.json` / `.yaml`, blocs SFC `<i18n>` | partiel : YAML non détecté, SFC ignorés | YAML détecté et validé (syntaxe `vue`) | **Blocs SFC `<i18n>`** : demandent un format multi-locale par composant, avec des clés préfixées pour éviter les collisions entre composants |
| Lingui | 2,6 M (`@lingui/core`) | 5 934 | PO `src/locales/{locale}/messages.po` (ICU dans `msgstr`) ; JSON `minimal` / `lingui` | PO OK ; JSON style `lingui` : `message` (texte source) traduit à la place de `translation` | OK : `translation` lu et écrit, `message`/`origin`/`comments` recopiés depuis la source pour les nouvelles entrées | — |
| Astro (i18n natif) | 11,5 M (`astro`) | — | dictionnaire de la recette `src/i18n/ui.ts` `{en:{…},es:{…}}` | absent (avertissement « hand-rolled i18n ») | OK : lu et patché comme dictionnaire TS, locales lues dans `astro.config` | — |
| Astro Starlight | 2,2 M | 9 530 | `src/content/i18n/{locale}.json` (UI) + pages Markdown | absent (stack « react », mauvais dossier) | UI OK (syntaxe i18next) | Pages `src/content/docs/{locale}/` : contenu Markdown, hors catalogue |
| `@angular/localize` | 2,0 M | (angular) | XLIFF 1.2 / 2.0 `src/locale/messages.{locale}.xlf`, XMB/XTB | XLIFF OK (`<x>`, `<ph>`, `<pc>`, ICU, notes) | OK | **XMB/XTB** non lus (format historique, `--format xmb`) ; extraction `$localize` / `i18n="…"` absente |
| VitePress | 2,0 M | 18 470 | dossiers de pages Markdown par locale | hors catalogue | hors catalogue | Traduction de contenu Markdown |
| Docusaurus | 2,0 M | — | `i18n/{locale}/code.json` + JSON thème/plugins, `{message, description}` | partiel : stack « react », syntaxe i18next, un seul fichier, cible neuve écrite en `defaultMessage` (illisible par Docusaurus), `description` perdue | OK : tous les JSON de `i18n/{locale}/`, champ `message` et `description` reproduits depuis la locale source, syntaxe ICU | La source `i18n/en/` n'existe qu'après `write-translations --locale en` (documenté) ; pluriels `usePluralForm` (`a\|b`) vérifiés comme du texte ; docs Markdown hors catalogue |
| ngx-translate | 1,9 M | 4 660 | `public/i18n/` ou `src/assets/i18n/{lang}.json`, `{{ x }}` | OK | OK | Extraction du pipe `\| translate` absente |
| Paraglide JS / inlang | 1,4 M | 2 006 (inlang) | `messages/{locale}.json` + `project.inlang/settings.json`, variantes `[{declarations, selectors, match}]` | **corruption** : `declarations` et `selectors` traduits (`"⟦de⟧ input count"`), stack `generic`, réglages ignorés | OK : nouveau format `inlang`, variantes à un sélecteur pluriel repliées en ICU (catégories CLDR de la cible ajoutées), squelette recopié depuis la locale de base, `pathPattern` et `locales` lus | Variantes à plusieurs sélecteurs : une entrée par `match`, sans ajout de catégories ; `baseLocale` ≠ `en` → `--source` (le `Detected` partagé n'a pas de champ source) |
| `@nuxtjs/i18n` | 0,98 M | 2 092 | `i18n/locales/{code}.json` / `.yaml` | OK (JSON) | OK, plus YAML et locales lues dans `nuxt.config` | — |
| Transloco | 0,49 M | 2 285 | `src/assets/i18n/{lang}.json` + scopes `{scope}/{lang}.json` | partiel : scopes ignorés, `langs` ignorés | OK : scopes `{ns}` (clés `scope:key`), `langs` lus | Les clés de scope s'écrivent `admin:users` dans glot (`admin.users` côté Transloco) : `extract` ne les rapproche pas |
| `@solid-primitives/i18n` | 0,29 M | 1 564 | dictionnaires TS, `{{ x }}` (`resolveTemplate`) | partiel : syntaxe ICU → `{{ name }}` rejeté comme erreur | OK (syntaxe i18next, fonctions conservées) | — |
| remix-i18next | 0,25 M | 710 | i18next (JSON ou `app/locales/{l}.ts`) | OK | OK | — |
| svelte-i18n | 0,17 M | 1 379 | `src/lib/i18n/locales/{locale}.json`, ICU | absent (dossier non trouvé, stack `generic`) | OK | — |
| ember-intl | 0,11 M | 438 | `translations/{locale}.yaml` (`en-us`), ICU | absent ; et une fois configuré, un pluriel ICU neuf était écrit en mapping `one:`/`other:` (perte de `=0`) | OK : détecté, et les pluriels restent des chaînes ICU en YAML sans racine de locale (correctif partagé avec Symfony `+intl-icu`) | — |
| typesafe-i18n / next-translate | ≈ 0,1 M | — | hors périmètre (voir `i18n-methods.md`) | — | — | — |
| sveltekit-i18n | 35 k | — | `src/lib/translations/{locale}/*.json`, `{{x}}` | absent | OK (syntaxe i18next) | Modificateurs `{{count; 0:…; default:…}}` vérifiés comme un simple placeholder |
| qwik-speak | 5 k | — | `i18n/{lang}/{asset}.json`, `{{x}}` | partiel : syntaxe ICU → `{{name}}` rejeté à la traduction | OK (syntaxe i18next) | — |

## Validation vérifiée (erreurs injectées)

| Cas | Résultat de `glot check` |
|---|---|
| i18next : `<1>` retiré | `tag_mismatch` |
| next-intl : `{name}` → `{nom}`, balise `<guidelines>` retirée | `placeholder_missing` + `placeholder_extra`, `tag_mismatch` |
| vue-i18n / Nuxt : 1 forme au lieu de 3, `@:hello` retiré, `{n}` perdu | `plural_category_missing`, `placeholder_missing` (lien), `placeholder_missing` |
| ngx-translate : `{{ name }}` → `{{ nom }}` | `placeholder_missing` + `placeholder_extra` |
| Lingui PO pl : catégorie `few` retirée | `plural_category_missing` |
| Angular XLIFF 1.2 : `<x id="INTERPOLATION"/>` et balises bold retirées | `placeholder_missing` ×3 |
| Angular XLIFF 2.0 : `<pc>` retiré | `tag_mismatch` |
| FormatJS extrait : `{name}` → `{nom}` | `placeholder_missing` + `placeholder_extra` |
| remix (TS) : `{{x}}` ajouté | `placeholder_extra` |
| ember-intl : `=0` perdu | `plural_category_missing` |

## Changements

- `crates/core/src/formats/inlang.rs` (nouveau) : format Paraglide/inlang. Il est choisi par chemin, sous `auto`, quand un
  ancêtre contient `project.inlang/settings.json` avec le plugin message-format. `Format::Inlang` a été ajouté à
  `catalog.rs` et `formats/mod.rs`.
- `crates/core/src/formats/json.rs` : le champ `translation` (Lingui) est prioritaire. Une nouvelle entrée de
  message-objet recopie l'entrée d'un fichier frère d'une autre locale (`i18n/{en}/…/navbar.json`), ce qui donne
  `message` + `description` pour Docusaurus et `message` + `origin` pour Lingui.
- `crates/core/src/formats/yaml.rs` : un pluriel neuf n'est écrit en mapping CLDR que dans un fichier Rails (racine de
  locale) ou dans un fichier qui contient déjà un mapping de ce type.
- `crates/cli/src/init_web.rs` (nouveau) : détection de Paraglide, Docusaurus, Starlight, ember-intl, svelte-i18n,
  sveltekit-i18n et qwik-speak, syntaxe pour `@solid-primitives/i18n`, scopes Transloco, et locales déclarées dans les
  configs (`langs`, `locales` sous forme de tableau, d'objets `code`, ou d'objet Starlight).
- `crates/cli/src/init.rs` : branchements de `init_web`. `find_in` accepte aussi `{locale}.yaml` / `.yml`, et
  `find_dictionary` accepte les fichiers placés sous un dossier `i18n/`.
- Tests : `formats::inlang::tests` (aller-retour octet pour octet, cible pl), `formats::json::tests::docusaurus_and_lingui_…`,
  `formats::js_web_tests` (YAML ICU, Rails), `init::web::tests` (11 stacks détectées, locales déclarées).
- Docs : section « JavaScript web libraries » de `web/content/docs/frameworks.md`.

## Prochains manques, par valeur

1. Blocs SFC `<i18n>` de vue-i18n (fort × M).
2. Extraction du code Angular (`$localize`, `i18n="…"`, pipe `translate`) et des templates `.vue` / `.svelte`, pour des
   « unused » fiables.
3. Un champ `source` dans `Detected`, pour prendre `baseLocale` (Paraglide), `defaultLocale` (Docusaurus, Nuxt) et
   `sourceLocale` (Lingui) au lieu de `en` par défaut.
4. Ordre des nouvelles clés calqué sur la source dans les cibles JSON.
5. XMB/XTB Angular (faible).
