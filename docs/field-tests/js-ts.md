# Field test : projets JS/TS (2026-10-18)

Les projets JS/TS du fondateur (Desktop, `~/VOYAGEURS`, `~/…`) ont été testés sur des copies. Les copies ont été faites avec
rsync, sans `node_modules`, `.git`, `.next`, `dist`, `build`, `.env*`, clés, secrets ni médias. Seul le provider `mock` a servi,
et aucun push n'a été fait. Les copies ont été supprimées ensuite. Les noms sont anonymisés (P01…P17), et ce document ne
contient que des structures et des comptes, jamais de contenu. P04 était une copie quasi identique de P01 : il est exclu.

Commandes par projet : `glot init --yes --provider mock`, `extract --json`, `i18nize --dry-run`, `status`, `check`. La chaîne
complète `i18nize --yes --no-install` → install → build a tourné sur 3 projets représentatifs : P13 (Vite), P17 (Next) et
P07 (CRA, i18next inline). Les trois builds passent : `tsc -b && vite build`, `next build` et `craco build`. Le build de P07
passe avec `DISABLE_ESLINT_PLUGIN=true`, à cause d'une erreur ESLint déjà présente dans le projet.

## Inventaire et résultats

Les colonnes « avant → après » donnent les résultats avant et après les correctifs ci-dessous. *Hard* = chaînes JSX en dur,
*cfg* = copie dans des objets de config/données, *missing* = clés utilisées absentes du catalogue.

| Projet | Stack | Fichiers | i18n existante | init (stack / source) | hard / cfg | missing avant → après | i18nize --dry-run | check |
|---|---|---|---|---|---|---|---|---|
| P01 | Next 15 App Router | 626 | **maison** : dictionnaires TS plats, 15 langues, ~6 000 clés | `next (no lib)`, catalogue introuvable → `TS/JS dictionaries`, 2 motifs | 2 898 / 9 455 | 4 223 → 11 | 106 s → 7,8 s ; refuse désormais (runtime propre) | **119 erreurs réelles** (placeholders `{x}` perdus en zh/ja/ko/hi…), 4 029 avertissements |
| P02 | Next App Router | 107 | aucune | source `en` → **`fr`** | 1 441 / 90 | 0 | 1 703 chaînes, 1 572 clés | — |
| P03 | Next App Router, **plusieurs root layouts** (route groups) | 599 | **maison** : 8 namespaces × 15 langues (`dictionaries/<ns>/<locale>.ts`) | `Pages Router` (faux) → App Router ; 8 motifs détectés | 1 130 / 664 | 409 → 13 | refuse (runtime propre) | **25 erreurs**, 88 clés orphelines |
| P05 | Next App Router | 252 | **maison** : objet TS imbriqué, accès par propriété (`t.common.home`) | `TS/JS dictionaries`, `en-US`/`zh-CN` | 133 / 114 | 0 | refuse | ✓ (257 clés lues) |
| P06 | Vite + React + Electron | 78 | **maison** : `src/i18n/{en,fr}.ts` + `t()` | `react-i18next` (faux) → `TS/JS dictionaries` | 47 / 2 | 317 → 0 | refuse | ✓ (395 clés) |
| P07 | CRA (craco), `.js` JSX | 71 | i18next, **ressources inline** dans `src/i18n.js` (5 langues) | catalogue proposé `public/locales/…` (inexistant) → `src/i18n.js` | 272 / 85 | 9 → 2 | 404 chaînes ; nouvelles clés écrites **dans** `src/i18n.js` | ✓, 24 clés orphelines (dérive réelle fr/en) |
| P08 | Vite + React Router | 70 | aucune | source → `fr` | 985 / 298 | 0 | 1 044 / 964 | — |
| P09 | Extension Chrome (Vite + CRXJS) | 11 | aucune | `en` | 26 / 10 | 0 | 39 / 40 | — |
| P10 | Extension Chrome MV3 vanilla | ~10 | `_locales/*/messages.json` (3 chaînes manifest) | `generic`, 0 fichier scanné | 0 | — | erreur « cannot detect the framework » | — |
| P11 | Vite + React + Tauri | 69 | aucune | `en` | 502 / 239 | 0 | 719 / 657 | — |
| P12 | Vite + React (généré par un agent) | 26 | aucune | source → `fr` | 144 / 535 (tables de symboles) | 0 | 156 / 155 | — |
| P13 | Vite + React Router + Tauri | 13 | aucune | source → `fr` | 126 / 29 | 0 | 164 / 162 ; **build ✓** | — |
| P14 | site HTML + JS vanilla | ~40 | aucune | `generic`, 0 fichier scanné | 0 | — | erreur framework | — |
| P15 | Vite + React | 18 | aucune | source → `fr` | 46 / 0 | 0 | 51 / 48 | — |
| P16 | exemple SDK React | 2 | aucune | `en` | 11 / 9 | 0 | 10 / 11 | — |
| P17 | Next 15 App Router minimal | 3 | aucune | `en` | 10 / 2 | 0 | 13 / 13 ; **build ✓** (next-intl, middleware) | — |

Temps (binaire debug) : `init` < 0,6 s, sauf P01 (3,4 s) ; `extract` ≤ 1,5 s ; `status` et `check` ≤ 1,8 s sur 15 × 6 000 clés.
Aucun crash.

**Chiffre marquant : 5 projets sur 15 ont leur propre i18n en dictionnaires TS/JS.** Avant les correctifs, glot les voyait
comme « 0 clé » et des milliers de clés « missing ». Il proposait aussi de greffer next-intl ou react-i18next par-dessus. Une
fois les dictionnaires lus, `glot check` trouve en 2 s des placeholders réellement cassés en production (P01 : 119, P03 : 25).

Échantillon manuel (~120 chaînes tirées au hasard dans 7 projets) : ~7 % de faux positifs. Ce sont des marques ou noms de
produit (« Apple Mail », raison sociale d'hébergeur, nom d'IDE), du technique (`GET /v1/whoami`) et des placeholders d'exemple
(`placeholder="postgres"`). Parmi les faux négatifs visibles : la copie dans des fichiers `.mjs`/vanilla hors `src/` (P10, P14)
et les dictionnaires lus par accès de propriété (P05).

## Bugs trouvés et corrigés

Chaque correctif a un test de non-régression construit sur un motif anonymisé.

| # | Symptôme réel | Cause | Correctif (test) |
|---|---|---|---|
| 1 | `i18nize` transformait des objets de config techniques en hooks et y traduisait des identifiants. Exemples : `db_host: 'postgres'` → `t("…postgres")`, `language: 'en'` → `t("sources.en")`, une config WebRTC `{ bundlePolicy: 'max-bundle', rtcpMuxPolicy: 'require' }` devenait `useIceConfig()`. Le comportement de l'app était cassé dès qu'une traduction existait | la règle « un mot en minuscules = identifiant » existait dans `extract` (scan) mais pas dans la réécriture (`refactor::has_copy`, propriétés non-UI et éléments de tableau d'un hook) | une seule règle `glot_extract::i18nize::looks_like_copy`, utilisée par le scan, `i18nize` (`config_message_of`) et `refactor` (`refactor::technical_defaults_are_not_config_copy`, `i18nize::config_hook_keeps_identifiers`) |
| 2 | `i18nize --dry-run` : 106 s sur P01 (626 fichiers, gros modules de données) | `config_hooks` cherchait chaque nom de hook dans tout le projet avec `contains` (configs × taille du projet) | les identifiants `use*` sont collectés une seule fois → 7,8 s (`refactor.rs`) |
| 3 | 6 projets sur 15 écrits en français étaient initialisés avec `source_locale = "en"`, donc `src/locales/en.json` se remplissait de français, avec `fr` en cible | la source valait toujours `en` quand aucun catalogue n'était trouvé | `init` scanne la copie avant de choisir la source et vote sur les mots-outils propres à chaque langue (`init::guess_language`). Les cibles par défaut deviennent `en, es, de` (`guesses_source_language_from_copy`) |
| 4 | Dictionnaires TS/JS faits main illisibles (P01, P03, P05, P06, P07) | aucun format de catalogue JS | nouveau format `Js` (`core/formats/js.rs`). Lecture : `export default {…}`, `export const fr: T = {…}`, `module.exports`, clés plates (`"nav.home"`) ou imbriquées, ressources i18next inline par langue avec le wrapper `translation`. Écriture **par patch** : valeur re-quotée en place, clé supprimée retirée avec sa ligne, nouvelles clés dans l'objet parent (style plat ou imbriqué suivi). Fonctions, spreads et types ne sont jamais touchés. Un nouveau fichier est écrit `export default {…};` (`js_dictionary_nested_round_trip`, `js_flat_dictionary_and_new_file`, `js_inline_i18next_resources`) |
| 5 | `init` ne trouvait pas ces dictionnaires et les scannait comme de la « copie en dur » | `find_catalogs` ne cherchait que du `.json` | `init::find_js_catalogs` : `{locale}.ts/js` dans un dossier i18n/locales/dictionaries (ou 3 fichiers de langue ou plus), un motif par namespace, ressources inline dans `src/i18n.{js,ts}`. Les fichiers trouvés sont ajoutés à `[extract].exclude` (`detects_ts_dictionaries`) |
| 6 | Sur un projet à runtime maison, `i18nize` aurait installé next-intl ou react-i18next en plus | le choix de bibliothèque en mode `auto` ignorait le runtime existant | refus explicite avec un message (« own i18n runtime … pass --lib to migrate ») (`tests/field_js.rs`, de bout en bout : init → extract → status → i18nize refusé → translate mock qui patche `fr.ts` → check) |
| 7 | CRA : `src/i18n.js` existant ignoré. Un second `src/i18n.ts` était créé, en conflit avec le premier, et l'entrée `src/index.js` n'était pas reconnue | `react::setup` ne regardait que `i18n.ts` et `main.*` / `index.tsx` / `index.jsx` | un `i18n.{js,jsx,tsx}` existant est conservé, sans nouvel import. `index.ts` et `index.js` sont acceptés comme entrée (`create_react_app_entry_and_existing_i18n_module`) |
| 8 | Next App Router avec plusieurs root layouts (route groups, sans `app/layout.tsx`) détecté comme **Pages Router**, avec risque de créer `pages/_app` | la détection exigeait `app/layout.*` | un dossier `app/` (ou `src/app/`) sans `pages/` suffit à reconnaître l'App Router. Note : « wrap each root layout » (`next_app_router_with_route_group_root_layouts`) |
| 9 | `extract --json` (et donc l'outil MCP) renvoyait `l&apos;instant` : un agent copiait l'entité telle quelle dans le catalogue | entités non décodées dans le scan (elles l'étaient déjà dans `i18nize`) | `decode_entities` partagé (`jsx_text_entities_are_decoded`) |
| 10 | `init` affichait « stack: react-i18next » sur une app sans aucune lib i18n | libellé | « react (no i18n library yet) » |

Fichiers modifiés : `crates/extract/src/{i18nize,scan,tests}.rs`, `crates/cli/src/{init,i18nize}.rs`,
`crates/cli/src/setup/{mod,react,next,refactor,tests}.rs`, `crates/core/src/{catalog.rs,formats/mod.rs,formats/js.rs,formats/tests.rs}`
et `crates/cli/tests/field_js.rs` (nouveau).

## Limites connues et cas non gérés

- **JS vanilla et extensions sans framework** (P10, P14) : `include` ne couvre que `src/`, `app/`, `components/`, `pages/` et `lib/`.
  `i18nize` exige React ou Next, et le format `_locales/*/messages.json` (chrome.i18n) n'est pas lu.
- **Dictionnaires lus par propriété** (`t.common.home`, P05) : les usages ne sont pas détectés, donc toutes les clés
  apparaissent « unused ».
- **Langue de référence ambiguë** : quand `en.ts` et `fr.ts` existent, `init` prend `en`. Or le code de P03 et P06 déclare `fr`
  comme référence, et dans P01 la source française est dans un autre fichier (`translations.ts`). Il faut passer `--source`.
- **Source multilingue** : dans P07, les ressources `en` sont en anglais mais la copie en dur est en français. `i18nize` range
  donc du français dans `en`.
- **Modules de données** (P01 : 9 455 chaînes « cfg » dans des jeux de données cuisine/météo/pays ; P12 : tables de symboles) :
  ils noient le rapport `config`.
- **Nouveau fichier de langue TS** : il est écrit en `export default {…}`. Le loader du projet doit l'importer (l'export nommé
  n'est pas recopié).
- Le scanner JS est fait main. Un littéral regex contenant une quote ou une accolade dans un dictionnaire le tromperait
  (cas jamais rencontré).
- **Monorepos** : aucun dans l'échantillon (le seul `pnpm-workspace.yaml` est à la racine de l'app), donc non testé.
- Balisage imbriqué (`Country <span>(Free)</span>`) : laissé tel quel, il faut `<Trans>` (4 occurrences dans P07).
- Avertissements bruyants : `i18nize` liste des constantes SCREAMING non UI des routes API (`app/api/**/route.ts`) comme
  « left as is ».

## Idées (classées par impact)

1. **`i18nize --lib custom`** pour envelopper les chaînes avec le `t()` de l'app (`--t-import "@/lib/i18n#t"`, signature
   `t(key)` ou `t(locale, key)`). Un tiers des projets a son propre runtime, et c'est aujourd'hui un refus.
2. **Usages par propriété** : `dict.section.key` / `t.section.key`, plus `keyof typeof fr` pour les clés typées. Les « unused »
   deviendraient fiables sur P05 et P06.
3. **Langue de référence** : lire `REFERENCE`, `fallbackLng` ou `defaultLocale` dans le code i18n pour choisir la source, et
   compléter avec le vote de langue sur les valeurs des catalogues.
4. **`glot check` multilingue** : avertir quand une valeur source n'est pas dans la langue source (P07), avec le détecteur de
   `init`.
5. **Glossaire amorcé** à l'init : marques et noms de produit en `do_not_translate` (nom du `package.json`, mots capitalisés
   répétés, raisons sociales). Cela supprime la majorité des faux positifs observés.
6. **Classer les « modules de données »** (beaucoup de chaînes, aucun JSX) à part dans `extract`, et orienter vers
   `glot labs db` / contenu au lieu de les compter comme UI.
7. **Ignorer le code serveur** (`app/api/**`, `route.ts`, `*.server.ts`) dans les warnings de hooks et le scan UI.
8. **`glot doctor`** (onboarding SaaS) : un résumé en une commande de la stack, du runtime détecté, des catalogues et des erreurs
   `check`. Sur P01, il aurait montré 119 placeholders cassés en 2 s, ce qui ferait une bonne démo de valeur au premier contact.
9. Format **chrome.i18n** (`_locales/<locale>/messages.json`, `description` → note, `$NAME$`) avec détection de
   `chrome.i18n.getMessage` et `__MSG_x__`. C'est un cas fréquent pour les extensions.
10. Inclure `*.mjs` dans les globs par défaut, et faire un repli `**/*.{js,ts}` (hors vendor/minifiés) quand aucun dossier
    standard n'existe.
