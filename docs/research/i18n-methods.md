# Méthodes i18n réellement utilisées — et ce que glot en adopte

> Recherche du 2026-10-18. Objectif : qu'un projet existant soit **adopté tel quel** (fichiers, clés, syntaxe et
> emplacement conservés, rien réécrit). Chiffres : registres publics, relevés le 2026-10-18 ; « support glot » vérifié
> en lisant le code et en lançant `target/integ/debug/glot` (`init --yes --provider mock`, `status`, `check`,
> `translate --yes`) sur 24 petits fixtures (`scratchpad/methods/`, hors repo).

## 0. Sources et limites des chiffres

| Source | Mesure | Période |
|---|---|---|
| `api.npmjs.org/downloads/point/last-week/<pkg>` | téléchargements npm hebdo | 2026-10-10 → 2026-10-16 |
| `api.github.com/repos/<o>/<r>` | étoiles | 2026-10-18 |
| `pypistats.org/api/packages/<pkg>/recent` | PyPI, 30 jours glissants | → 2026-10-18 |
| `packagist.org/packages/<v>/<p>.json` | Packagist, 30 jours + total | → 2026-10-18 |
| `crates.io/api/v1/crates/<c>` | crates.io, 90 jours + total | → 2026-10-18 |
| `rubygems.org/api/v1/gems/<g>.json` | RubyGems, total | 2026-10-18 |
| `pub.dev/api/packages/<p>/score` | pub.dev, 30 jours + likes | → 2026-10-18 |
| `azuresearch-usnc.nuget.org/query` | NuGet, total | 2026-10-18 |

- **Enquêtes** : aucune donnée d'usage i18n trouvée. State of React 2024 (7 870 réponses) et 2025 n'ont pas de question
  i18n visible ; State of JS et Stack Overflow ne posent pas la question à notre connaissance. Weblate qualifie PO de
  « format le plus utilisé du logiciel libre » sans pourcentage. Aucune part de marché publiée par Crowdin/Lokalise/Phrase.
- Les téléchargements comptent la CI et les dépendances transitives (ex. `intl-messageformat` 12,8 M/sem. vient
  surtout de react-intl/FormatJS ; `babel` PyPI vient aussi de Sphinx). Les plateformes natives (Spring
  `messages.properties`, .NET resx, iOS, Android, Qt) n'ont pas de paquet à compter : leur part est déduite de la
  taille de la plateforme. Les « parts » ci-dessous sont donc des **ordres de grandeur relatifs**, pas des parts de marché.

## 1. Bibliothèques par écosystème

### JS / TS (npm, semaine du 2026-10-10 au 2026-10-16)

| Bibliothèque | dl/sem. | Étoiles GitHub | Format de catalogue par défaut | Syntaxe |
|---|---:|---:|---|---|
| i18next (cœur) | 13 869 475 | 8 645 | JSON imbriqué, `public/locales/{lng}/{ns}.json` | i18next v4 (`{{x}}`, `_one/_other`, `$t()`) |
| react-i18next | 10 343 106 | 10 054 | idem | idem + `<Trans>` `<0>…</0>` |
| i18next-browser-languagedetector | 3 896 660 | — | (détection) | — |
| i18next-http-backend | 1 531 046 | — | charge `/locales/{{lng}}/{{ns}}.json` | — |
| i18next-cli / i18next-parser | 494 911 / 298 421 | — | extracteurs officiels | — |
| i18next-icu | 413 219 | — | i18next avec messages ICU | ICU |
| next-i18next | 342 263 | 6 162 | `public/locales/{lng}/{ns}.json` | i18next |
| remix-i18next | 104 975 | — | `public/locales/{lng}/{ns}.json` | i18next |
| intl-messageformat (FormatJS) | 12 815 819 | 14 741 (formatjs) | — | ICU MF1 |
| @formatjs/intl | 1 929 614 | | | ICU |
| react-intl | 1 774 423 | | `lang/{locale}.json` plat, ids explicites ou hash (`formatjs extract`) | ICU, `<b>…</b>` |
| next-intl | 4 256 326 | 4 382 | `messages/{locale}.json` imbriqué | ICU, `t.rich` tags |
| vue-i18n | 2 152 369 | 2 712 | `src/locales/{locale}.json` (ou SFC `<i18n>`, YAML) | `{name}`, `{0}`, pluriels `a \| b \| c`, liens `@:key` |
| @intlify/unplugin-vue-i18n | 605 731 | | | |
| @nuxtjs/i18n | 418 621 | 2 085 | `i18n/locales/{code}.json` (v9 : dossier `i18n/`) | vue-i18n |
| @lingui/core / @lingui/react | 1 059 812 / 797 580 | 5 918 | **PO** `src/locales/{locale}/messages.po` (msgid = texte source ou id hash), ou JSON | ICU dans msgstr |
| @angular/localize | 771 595 | (Angular) | **XLIFF 1.2** `src/locale/messages.{locale}.xlf` (ou XLIFF 2, XMB, JSON, ARB) | ICU, `<x id="INTERPOLATION"/>` |
| @ngx-translate/core | 756 212 | 4 658 | `src/assets/i18n/{lang}.json` (Angular 17+ : `public/i18n/`) | `{{ param }}` |
| @jsverse/transloco (+ @ngneat 42 464) | 177 991 | 2 284 | `src/assets/i18n/{lang}.json`, scopes | `{{ param }}` |
| @inlang/paraglide-js | 518 955 | 2 006 (inlang) | `messages/{locale}.json` + `project.inlang/settings.json` | `{name}`, variantes |
| @astrojs/starlight | 884 121 | — | `src/content/i18n/{locale}.json` (UI) | i18next |
| expo-localization | 1 935 223 | — | (détection de locale, souvent + i18n-js ou i18next) | — |
| i18n-js | 339 164 | 240 | JSON/JS `{ en: {…} }` | `%{name}`, objets `{one, other}` |
| react-native-localize | 295 620 | 2 443 | (détection) | — |
| node-polyglot | 243 257 | 3 722 | JSON | `%{name}`, pluriels `a \|\|\|\| b` |
| svelte-i18n | 70 527 | 1 380 | `src/lang/{locale}.json` | ICU (intl-messageformat) |
| @tolgee/react | 52 258 | 4 132 (plateforme) | JSON | ICU |
| intlayer | 53 229 | — | fichiers `*.content.ts` par composant | propre |
| gt-next | 49 092 | — | — | — |
| next-translate | 40 336 | — | `locales/{lang}/{ns}.json` | `{{x}}`, `_one/_other` |
| typesafe-i18n | 34 777 | 2 479 | `src/i18n/{locale}/index.ts` | `{name}`, `{{singulier\|pluriel}}` |
| rosetta | 26 585 | — | objet JS | `{{name}}` |
| sveltekit-i18n | 13 130 | — | JSON | `{{x}}` |

Contexte frameworks (même semaine) : react 115,4 M, next 40,8 M, vue 10,3 M, react-native 9,0 M, expo 6,9 M,
svelte 4,1 M, @angular/core 3,4 M, @sveltejs/kit 2,0 M, nuxt 1,4 M.

**Lecture** : i18next + react-i18next dominent (13,9 M/sem. pour le cœur, ≈ 3× next-intl, ≈ 6× vue-i18n), puis FormatJS/react-intl, next-intl, vue-i18n, Lingui, Angular (localize ≈ ngx-translate),
paraglide. Tout le reste est < 0,4 M/sem.

### Mobile

| Méthode | Usage mesuré | Format |
|---|---|---|
| iOS String Catalog `.xcstrings` (Xcode 15+) | défaut des nouveaux projets Xcode | JSON, toutes locales dans 1 fichier, `%@ %lld`, variations plural/device |
| iOS `.strings` + `.stringsdict` | legacy, encore massif | `"key" = "value";` (clé = texte source souvent), UTF-16 |
| SwiftGen / R.swift | 9 543 / 9 547 étoiles | génèrent du code depuis `.strings`/`.xcstrings` (n'imposent pas de format) |
| Android `res/values-*/strings.xml` | toute app Android | `%1$s`, `<plurals>`, `<string-array>` |
| Flutter `intl` + gen-l10n (ARB) | `intl` 12 476 919 dl/30 j, 6 110 likes | `lib/l10n/app_{locale}.arb`, ICU |
| easy_localization | 311 359 dl/30 j, 3 843 likes | `assets/translations/{locale}.json`, `{}` positionnel, `{name}`, pluriels `{zero, one, other}` en sous-objet |
| slang | 336 806 dl/30 j, 794 likes | `lib/i18n/strings_{locale}.i18n.json` (ou yaml/csv), `${name}` / `{name}`, pluriels en sous-objet, `@:key` |
| GetX `.tr` | `get` 757 734 dl/30 j (pas que i18n) | Map Dart `Map<String, Map<String,String>>` |
| React Native | i18n-js 339 k/sem., react-native-localize 296 k/sem., expo-localization 1,9 M/sem. | JSON i18next ou i18n-js |

### Back-end / desktop

| Écosystème | Usage mesuré | Format / convention |
|---|---|---|
| Ruby / Rails I18n | gem `i18n` 1 457 391 092 dl total ; `rails-i18n` 143,9 M ; `i18n-tasks` 34,4 M | `config/locales/**/*.{locale}.yml`, racine = locale, `%{name}`, `one:/other:` |
| Ruby gettext | `fast_gettext` 71,7 M, `gettext` 61,8 M total | PO |
| Python / Django / Flask | `babel` 101 311 977 dl/30 j ; `flask-babel` 5,96 M ; `polib` 4,86 M ; django 41,1 M | `locale/{lang}/LC_MESSAGES/django.po`, `%(name)s`, `{name}` (python-brace-format) |
| PHP / Symfony | `symfony/translation` 22 931 021 dl/30 j (954 M total) | `translations/messages.{locale}.{xlf\|yaml}`, `messages+intl-icu.{locale}.yaml` (ICU), `%name%` legacy |
| PHP / Laravel | `laravel/framework` 18 234 938 dl/30 j ; `spatie/laravel-translatable` 1,85 M ; `mcamara/laravel-localization` 259 k | `lang/{locale}/*.php` (tableaux) + `lang/{locale}.json` (clé = texte source), `:name`, pluriels `{0} …\|[1,*] …` |
| PHP gettext | `gettext/gettext` 940 866 dl/30 j | PO |
| Java / Spring / Jakarta | plateforme (non mesurable par paquet) | `messages_{locale}.properties` (`ResourceBundle`), MessageFormat `{0}`, `''`, ChoiceFormat |
| .NET | `Microsoft.Extensions.Localization` 615,4 M dl total ; `.Abstractions` 729,8 M | `Resources/*.{culture}.resx` (XML), `{0}` composite format |
| Go | `nicksnyder/go-i18n` 3 548 étoiles ; `golang.org/x/text` | `active.{lang}.toml\|json\|yaml`, `{{.Name}}`, `one/other` |
| Rust | `fluent-bundle` 7 574 215 dl/90 j (21,4 M total) ; `fluent` 6,4 M/90 j ; `i18n-embed` 4,07 M/90 j ; `rust-i18n` 1,47 M/90 j ; `gettext-rs` 649 k/90 j | Fluent `.ftl` (`{ $name }`, sélecteurs) ; rust-i18n `locales/*.yml` (`_version: 2` : toutes locales dans un fichier), `%{name}` |
| Python Fluent | `fluent.runtime` 436 614 dl/30 j | `.ftl` |
| Qt | plateforme | `.ts` (XML, `%1`, `<numerusform>`), compilé en `.qm` |
| Unity | Localization package officiel | String Tables (assets YAML), import/export CSV/XLIFF, Smart Strings |
| WebExtensions (Chrome/Firefox) | toute extension localisée | `_locales/{locale}/messages.json` `{key:{message, description, placeholders}}`, `$NAME$` |

## 2. Conventions de clés et de fichiers

| Convention | Exemples | Qui |
|---|---|---|
| Clés pointées, JSON imbriqué | `home.title` ↔ `{"home":{"title":…}}` | next-intl, i18next, vue-i18n, ngx-translate, Rails/Symfony YAML |
| Clés plates (avec ou sans points) | `{"home.title": …}`, `{"HOME_TITLE": …}` | react-intl, ARB (pas de points), Android (`home_title`), resx, properties |
| Namespaces = fichiers | `public/locales/en/common.json` + `t('common:home.title')`, `useTranslation('home')` | i18next/next-i18next/remix-i18next, next-translate ; Symfony domaines `messages`/`validators` ; gettext domaines ; Rails `devise.en.yml` |
| Texte source comme clé | gettext `msgid`, iOS `.strings` (`"Save" = "Save";`), Laravel `lang/fr.json`, i18next `keySeparator:false` + `nsSeparator:false`, Lingui `t\`Hello\`` | gettext, iOS, Laravel, Lingui, Django |
| Ids générés / hash | FormatJS `[sha512:contenthash:base64:6]`, Lingui ids hash (`Gq3yVQ`), Angular `@angular/localize` (id = hash du message ou `@@customId`) | FormatJS, Lingui, Angular |
| Contexte | i18next `key_male`/`key_female` (`context`) ; gettext `msgctxt` ; Qt `<context>` ; Lingui/FormatJS `context`/`description` | |
| Pluriel par suffixe de clé | i18next v4 `key_one/_few/_many/_other`, `_zero`, `_ordinal_one` ; v3 legacy `key_plural`, `key_0` | i18next, next-translate |
| Pluriel en sous-objet | `inbox: {one:…, other:…}` | Rails, i18n-js, easy_localization, slang, go-i18n |
| Pluriel dans la valeur | ICU `{n, plural, …}` ; vue-i18n `a \| b` ; polyglot `a \|\|\|\| b` ; Laravel `{0}…\|[1,*]…` ; Fluent sélecteurs | |
| Pluriel structurel du format | gettext `msgid_plural/msgstr[n]`, Android `<plurals>`, `.stringsdict`, xcstrings variations, Qt `<numerusform>` | |

Emplacements usuels : `messages/{locale}.json` (next-intl, paraglide), `public/locales/{lng}/{ns}.json` (i18next),
`src/locales/{locale}.json` (vue, lingui JSON), `src/locales/{locale}/messages.po` (lingui), `lang/{locale}.json`
(react-intl, Laravel), `lang/{locale}/*.php` (Laravel), `src/assets/i18n/` et `public/i18n/` (ngx-translate,
Transloco), `src/locale/messages.{locale}.xlf` (Angular), `i18n/locales/` (Nuxt 9), `config/locales/` (Rails),
`translations/` (Symfony), `locale/{lang}/LC_MESSAGES/` (Django/gettext), `src/main/resources/` (Spring),
`Resources/` (.NET), `res/values-*/` (Android), `*.lproj/` ou `*.xcstrings` (iOS), `lib/l10n/` (Flutter gen-l10n),
`assets/translations/` (easy_localization), `_locales/{locale}/` (WebExtensions).

## 3. Interpolation et pluriels par bibliothèque

| Lib | Variable | Pluriel | Imbrication / balises |
|---|---|---|---|
| i18next | `{{name}}`, `{{ name }}`, `{{val, number}}`, `{{- raw}}` | clés `_one/_other` (+ `_few/_many` selon CLDR), `count` | `$t(key)`, `$t(key, {"x":1})`, `<0>…</0>` / `<b>` (Trans) |
| ngx-translate / Transloco | `{{ value }}` | aucun natif (messageformat optionnel) | — |
| ICU MF1 (next-intl, FormatJS, Lingui, svelte-i18n, Angular, ARB, Symfony `+intl-icu`) | `{name}`, `{n, number}` | `{n, plural, one {#} other {#}}`, `selectordinal`, `select` | `<b>…</b>` (next-intl, react-intl) ; Angular `<x id="…"/>` |
| MessageFormat 2 | `{$name}` | `.match {$n :number} one {{…}} * {{…}}` | `{#b}…{/b}` |
| vue-i18n | `{name}`, `{0}`, `{'@'}` littéral | `zéro \| un \| {count} plusieurs` (choix par `\|`) | `@:key`, `@.lower:key` |
| polyglot / i18n-js / Rails | `%{name}` | polyglot `\|\|\|\|` ; i18n-js et Rails sous-objet `one/other` | — |
| Laravel | `:name`, `:Name`, `:NAME` | `{0} aucun\|{1} un\|[2,*] :count` ou `un\|plusieurs` | — |
| Symfony legacy | `%name%` | `{0}…\|]1,Inf[…` | — |
| gettext | `%s`, `%d`, `%(name)s` (python-format), `{name}` (python-brace-format), `%1$s` | `msgid_plural` + `Plural-Forms` | — |
| iOS / Android | `%@`, `%lld`, `%1$@` / `%1$s`, `%d` | stringsdict / xcstrings variations ; `<plurals>` | Android HTML `<b>`, CDATA |
| Java MessageFormat / .NET | `{0}`, `{0,number}` (Java : `'` échappe, `''` = apostrophe) ; `{0:N2}` | Java ChoiceFormat `{0,choice,0#…\|1#…}` ; .NET aucun | — |
| Fluent | `{ $name }`, `{ -brand }` (terme) | `{ $n -> [one] … *[other] … }` | attributs `.title` |
| go-i18n | `{{.Name}}` (text/template) | `one/other/…` par message | — |
| Qt | `%1`, `%n` | `<numerusform>` | — |
| Chrome i18n | `$NAME$` (déclaré dans `placeholders`), `$1` | aucun | — |
| typesafe-i18n | `{name}`, `{0}` | `{{chat\|chats}}` | — |
| easy_localization | `{}` positionnel, `{name}` | sous-objet `zero/one/two/few/many/other` | `@:key` |

## 4. Matrice : méthode × usage × support glot × effort

Support : **OK** (adopté tel quel, round-trip et validation), **partiel**, **absent**. Effort : S ≤ 1 j, M 2-4 j, L 1-2 sem.
Preuves : lecture de `crates/cli/src/{init.rs,init_native.rs}`, `crates/core/src/{catalog.rs,formats/*,validate.rs}`,
`crates/extract/src/{lib.rs,scan.rs}` + exécution du binaire sur les fixtures.

| Méthode | Usage (relatif) | Détection `init` | Lecture/écriture | Validateur | Extraction code | Verdict | Effort |
|---|---|---|---|---|---|---|---|
| next-intl JSON imbriqué | très fort | OK | OK | OK (ICU) | OK (`useTranslations`, `t.rich`) | **OK** | — |
| react-intl / FormatJS JSON plat `{id: msg}` | fort | OK (`lang/`) | OK | OK | OK (`formatMessage`, `<FormattedMessage>`) | **OK** | — |
| FormatJS format extrait `{id:{defaultMessage,description}}` | moyen | OK | **mauvais** : `description` traduite comme un message | — | — | partiel | S |
| i18next JSON, clés pointées, `{{x}}`, `$t()`, `<1>` | très fort | OK | OK | OK | OK (`useTranslation`, `keyPrefix`, `<Trans i18nKey>`) | OK | — |
| i18next pluriels v4 `_one/_other` → locales à `few/many` | très fort | — | n'ajoute **pas** `key_few`/`key_many` (pl testé : 0 erreur, 0 avertissement) | aucun contrôle de famille | OK (base reconnue) | **partiel (faux vert)** | M |
| i18next namespaces (plusieurs fichiers/locale) | fort | OK (1 pattern par fichier) | clés fusionnées sans préfixe : `common.json:title` et `home.json:title` → **une seule clé**, `home:title` jamais traduite | — | `ns:` supprimé (`strip_ns`) | partiel (perte silencieuse) | M |
| i18next contexte `_male/_female` | moyen | — | traduit comme clés séparées | — | marqué « unused » | partiel | S |
| Valeurs JSON non-string (tableaux `returnObjects`, nombres, booléens, null) | fort (i18next, ngx, vue) | — | **erreur bloquante** : `key list has a array value` → `status`/`check`/`translate` refusent le projet entier | — | — | **absent (bloquant)** | S |
| Texte source comme clé avec ponctuation (`keySeparator:false`) | moyen | — | **corrompt** : `"Welcome back."` écrit `{"Welcome back": {"": …}}` en cible | — | — | **absent (corruption)** | S |
| vue-i18n JSON | fort | OK | OK | ICU au lieu de vue : pluriels `\|` non vérifiés (2 formes au lieu de 3 acceptées), `@:hi` supprimé accepté | `$t` dans `<script>` seulement ; `<template>` ignoré → faux « unused » | partiel | M |
| Nuxt i18n (`i18n/locales/`) | moyen | absent (dossier non listé) | OK si configuré | idem vue | idem | partiel | S |
| ngx-translate / Transloco `src/assets/i18n/`, `public/i18n/` | moyen | **absent** (stack `generic`, `locales/`) | OK si configuré | `{{ x }}` OK en syntaxe i18next (init choisit ICU) | `translate` pipe / `transloco` absents | partiel | S |
| Angular `@angular/localize` XLIFF 1.2/2.0 | moyen | absent | absent | ICU OK | `$localize`, `i18n="…"` absents | **absent** | L |
| Lingui PO (ICU dans msgstr) | moyen | **faux** (init propose `src/locales/{l}.json`) | lit OK ; **réécrit l'ICU en `msgid_plural/msgstr[n]`** → casse Lingui | ICU OK | `t\`…\``, `<Trans>` macro absents | partiel (corruption) | S-M |
| Lingui JSON (`format: "minimal"`) | faible | OK | OK | OK | absent | partiel | — |
| paraglide / inlang message format | moyen | `generic` (trouve `messages/`) | OK, mais `$schema` traité comme une clé (avertissement « untranslated ») ; variantes v2 non gérées | ICU (≈ OK) | absent | partiel | S |
| svelte-i18n / Astro Starlight / typesafe-i18n / rosetta | faible | dossiers parfois trouvés | JSON/TS OK | syntaxe à choisir à la main | absent | partiel | S |
| polyglot / i18n-js `%{x}` | faible-moyen | `generic` → ICU | OK | `%{x}` vérifié par chance (ICU voit `{x}`), `\|\|\|\|` et sous-objets pluriels non vérifiés | absent | partiel | S |
| TS/JS dictionnaires (`locales/fr.ts`) | moyen | OK | OK (patch) | selon syntaxe | — | OK | — |
| iOS `.xcstrings` | fort (mobile) | OK | OK (round-trip, variations) | OK (printf) | absent (Swift) | OK | — |
| iOS `.strings`/`.stringsdict` (UTF-16) | fort | OK (`*.lproj`) | OK | OK | absent | OK | — |
| Android `strings.xml` | fort | OK (`values-` qualifiers) | OK (`<plurals>`, arrays, `translatable=false`) | OK | absent (Kotlin/Java) | OK | — |
| Flutter gen-l10n ARB | fort | OK (`l10n.yaml` ou `pubspec`) | OK | OK | absent (Dart) | OK | — |
| Flutter easy_localization / slang JSON | moyen | **absent** (`assets/translations/` non cherché) | JSON lu, mais pluriels en sous-objet traités comme clés (pas de `few/many` ajoutés) | `{}` positionnel non compris | absent | partiel | S-M |
| gettext PO (Django, Flask, PHP, C) | fort | OK (`LC_MESSAGES`) | OK (msgctxt, fuzzy, Plural-Forms) | `%(name)s`, `{name}` OK | absent (Python `_()`) | OK | — |
| Rails YAML `config/locales/{l}.yml` | fort | OK | OK (pluriels `one/other` complétés selon CLDR) | OK (`%{x}`) | absent (Ruby) | OK | — |
| Rails multi-fichiers `*.{l}.yml` (`devise.en.yml`) | fort | seulement `{locale}.yml` | avec pattern ajouté à la main : clé lue, mais écrite dans `fr.yml` au lieu de `devise.fr.yml` | OK | — | partiel | S |
| Symfony YAML (`messages+intl-icu.{l}.yaml`) | fort | absent (stack `generic`) | OK si configuré | ICU OK ; `%name%` legacy non | absent | partiel | S |
| Symfony XLIFF | fort | absent | absent | — | — | **absent** | M (partagé avec Angular) |
| Laravel `lang/{l}.json` | fort | trouvé (dossier `lang`) mais stack `generic`, syntaxe ICU | OK | `:name` → `:nom` **accepté** | absent (PHP `__()`) | partiel | S |
| Laravel `lang/{l}/*.php` | fort | absent | absent | — | — | **absent** | M |
| Java/Spring `.properties` | fort | absent | absent | ICU proche mais `''` Java ≠ ICU | absent | **absent** | M |
| .NET `.resx` | fort | absent | absent | `{0}` OK en ICU | absent | **absent** | M |
| WebExtensions `_locales/{l}/messages.json` | moyen | absent | JSON lu mais `description` traduite | `$NAME$` absent | — | absent | S |
| Go go-i18n TOML/JSON | faible-moyen | absent | absent (TOML) | `{{.Name}}` absent | absent | absent | M |
| Fluent `.ftl` (Rust, Firefox, Python) | moyen (Rust) | absent | absent | absent | absent | absent | L |
| rust-i18n YAML `_version: 2` | faible | absent | absent (toutes locales dans un fichier) | `%{x}` OK | absent | absent | M |
| Qt `.ts` | faible-moyen | absent | absent | `%1` absent | absent | absent | M |
| Unity String Tables / CSV | faible | absent | absent | Smart Strings absent | absent | absent | L |
| MF2 | émergent | — | (JSON) | OK | — | OK | — |

## 5. Top 15 des manques à combler

Classement = part d'usage × (1 / effort), bugs d'adoption bloquants d'abord (un projet i18next qui plante à
l'ouverture n'est pas « adopté »).

1. **Valeurs JSON non-string tolérées** (i18next, ngx-translate, vue — très fort, S).
   `catalog.rs::flatten` : tableaux, nombres, booléens, `null` ne doivent plus faire échouer le workspace. Lecture :
   ignorer (ou exposer les tableaux de chaînes en `key.0`, `key.1` comme Android) ; écriture : `merge_into` les garde
   verbatim (déjà le cas pour les clés non touchées) ; `to_json` ne doit pas les perdre. Test : fixture `arr/`.

2. **Pluriels i18next v4 par suffixe** (i18next — très fort, M).
   Grouper `base_{zero,one,two,few,many,other}` (et `base_ordinal_*`) en une famille ; à la traduction, générer les
   catégories CLDR de la cible (pl : `_one/_few/_many/_other`), supprimer celles qu'elle n'a pas (ja : `_other` seul) ;
   validateur : erreur `plural_category_missing` au niveau famille ; plan/`status` comptent la famille. Le plus simple :
   replier la famille en ICU `{count, plural, …}` à la lecture et la déplier à l'écriture, comme les formats natifs
   (`formats::icu_choice`). `_zero` reste optionnel ; legacy v3 `_plural` détecté via `compatibilityJSON`.

3. **Ne jamais re-découper une clé qui ne l'était pas** (texte source comme clé — moyen, S, corruption).
   `Catalog::write`/`merge_into`/`insert_nested` : une clé lue à plat (`"Welcome back."`) doit être réécrite à plat ;
   détecter `keySeparator: false` / `nsSeparator: false` dans `i18n.{js,ts}` (et `lang/{l}.json` Laravel, toujours
   plat) → `format = "json-flat"`. Interdire l'écriture d'une clé finissant par `.` en mode imbriqué.

4. **Namespaces i18next préservés** (i18next/next-i18next/remix — fort, M).
   Placeholder `{ns}` dans `files` (`public/locales/{locale}/{ns}.json`) → clé interne `ns:key` (séparateur configurable),
   le fichier de destination est déduit du namespace ; `init` génère ce pattern au lieu d'un pattern par fichier.
   Extraction : ne plus `strip_ns` mais résoudre `useTranslation('home')`, `t('common:x')`, `defaultNS`. Évite la
   collision silencieuse observée (`home:title` jamais traduite). Le même mécanisme couvre next-translate, Symfony
   (domaines), Rails `devise.{l}.yml` et gettext (domaines).

5. **Lingui PO : garder l'ICU dans `msgstr`** (Lingui — moyen, S).
   `po.rs::write` : si la source n'a pas `msgid_plural` et que la valeur est ICU, l'écrire telle quelle (aujourd'hui
   transformée en `msgid_plural/msgstr[n]`, illisible par Lingui). `init` : `@lingui/*` + `lingui.config.{js,ts}` →
   lire `catalogs[].path` (défaut `src/locales/{locale}/messages`) et `format` (po / json) ; syntaxe `icu`. Extraction :
   macros `` t`…` ``, `msg`, `<Trans>…</Trans>` (id = hash Lingui, ou texte).

6. **ngx-translate / Transloco** (Angular — moyen, S).
   `init` : dépendances `@ngx-translate/core`, `@jsverse/transloco`, `@ngneat/transloco` → dossiers
   `src/assets/i18n/`, `public/i18n/`, `assets/i18n/{scope}/` ; syntaxe `i18next` (parseur `{{ x }}` déjà tolérant aux
   espaces) sans les règles `_one/$t`. Extraction : pipe `'KEY' | translate`, `| transloco`, `translate.instant('KEY')`,
   directive `[translate]` dans les templates `.html`.

7. **Syntaxe vue-i18n + templates Vue** (vue-i18n/Nuxt — fort, M).
   `Syntax::Vue` : `{name}`, `{0}`, littéraux `{'@'}`, liens `@:key` / `@.lower:key` (même nombre et mêmes clés exigés,
   comme `$t()` i18next), pluriels `a | b | c` (nombre de branches identique à la source, `{count}`/`{n}` par branche).
   `init` : `vue-i18n`/`@nuxtjs/i18n` → syntaxe `vue`, dossiers `i18n/locales/` et `locales/` ; lire `nuxt.config`
   `i18n.langDir`. Extraction : `$t(`/`t(` dans `<template>` (et `v-t`, `<i18n-t keypath>`) pour supprimer les faux
   « unused ».

8. **Laravel : syntaxe `:name` + `lang/{l}/*.php`** (Laravel — fort, M).
   `init` : `composer.json` avec `laravel/framework` → stack `laravel`, syntaxe `laravel`, fichiers `lang/{locale}.json`
   (plat, clé = texte) + `lang/{locale}/*.php` (ou `resources/lang/` avant Laravel 9). Format `php` : scanner de
   `return [ 'k' => 'v', 'nested' => [...] ];` sur le modèle de `formats/js.rs` (patch en place, quotes PHP).
   Validateur : `:name`/`:Name`/`:NAME` comme placeholders, pluriels `{0} …|{1} …|[2,*] …` et `a|b`.
   Extraction PHP/Blade : `__('…')`, `trans('…')`, `@lang('…')`, `trans_choice`.

9. **XLIFF 1.2 / 2.0** (Angular `@angular/localize`, Symfony, exports Xcode/TMS — fort, M-L).
   Format `xliff` sur `formats/xml.rs` : `trans-unit@id` (ou `@resname`) → clé, `<source>` = texte source implicite,
   `<target state>` (`needs-translation` → stale, `<note>` → note) ; placeholders `<x id equiv-text>`, `<ph>`, `<pc>`
   traités comme balises dans le validateur (même ensemble exigé). Écriture : patch des `<target>`. `init` :
   `angular.json` (`i18n.sourceLocale`, `locales.{l}.translation`) et `translations/*.xlf` Symfony.

10. **Java / Spring `.properties`** (Spring — fort, M).
    Format `properties` : `key=value`/`key: value`, continuations `\`, commentaires `#`/`!`, échappements `\uXXXX`,
    encodage ISO-8859-1 par défaut (UTF-8 si Spring `spring.messages.encoding` ou fichier déjà UTF-8), patch en place.
    `init` : `src/main/resources/messages{_locale}.properties` (source = fichier sans suffixe), `pom.xml`/`build.gradle`.
    Syntaxe `java` : MessageFormat `{0}`, `{0,number}`, `''` = apostrophe, ChoiceFormat.

11. **.NET `.resx`** (.NET — fort, M).
    Format `resx` sur `xml.rs` : `<data name><value>` + `<comment>` → note, `Strings.resx` = source,
    `Strings.{culture}.resx` = cibles (ajouter le placeholder de nom de fichier `.{locale}` absent pour la source,
    comme `android_values`). Syntaxe `dotnet` : `{0}`, `{0:N2}`, `{{` échappé. `init` : `*.csproj` + `*.resx`.

12. **Paraglide / inlang** (paraglide — moyen, S).
    `init` : `project.inlang/settings.json` → `baseLocale`, `locales`, `plugin.inlang.messageFormat.pathPattern`.
    Ignorer les clés `$schema` (déjà conservées à l'écriture, mais comptées comme message). Format v2 : valeurs en
    tableau de variantes (`[{"match": {...}, ...}]`) gardées verbatim ou repliées en ICU `select/plural`.

13. **Pluriels en sous-objet dans JSON + Flutter easy_localization / slang / i18n-js** (moyen, S-M).
    Généraliser la règle YAML (« mapping dont les enfants sont uniquement des catégories CLDR = pluriel ») au JSON :
    `{"money": {"zero":…, "one":…, "other":…}}` → ICU à la lecture, catégories de la cible à l'écriture (comme
    Rails, déjà OK). `init` : `easy_localization` dans `pubspec.yaml` → `assets/translations/{locale}.json` ;
    `slang` → `lib/i18n/*_{locale}.i18n.json` ; `i18n-js`. Syntaxe : `{}` positionnel (easy_localization), `${x}` (slang).

14. **JSON à champ message** (FormatJS extrait, WebExtensions — moyen, S).
    Reconnaître `{key: {defaultMessage|message, description, placeholders}}` : le message est la valeur, `description`
    devient note (jamais traduite — aujourd'hui elle l'est), le reste verbatim. `init` : `manifest.json` +
    `_locales/{l}/messages.json` ; syntaxe Chrome `$NAME$` (doit exister dans `placeholders`).

15. **Détection back-end restante : Symfony, Rails multi-fichiers, Nuxt, Django `_()`** (fort, S chacun).
    `init_native.rs` : `composer.json` + `symfony/translation` → `translations/{domaine}.{locale}.{yaml,xlf}` et
    syntaxe ICU si `+intl-icu`, `%name%` sinon (ajouter cette syntaxe au validateur) ; Rails → pattern
    `config/locales/**/*{locale}.yml` + écrire une clé nouvelle dans le fichier cible **miroir** du fichier source
    (`devise.en.yml` → `devise.fr.yml`, aujourd'hui `fr.yml`). Extraction hors JS (ponytail : regex par langage suffit
    pour les usages, pas un parseur) : Python `_()`/`gettext()`, Ruby `t('.x')`, PHP `__()`, Swift
    `String(localized:)`/`NSLocalizedString`, Kotlin `R.string.x`/`getString`, Dart `AppLocalizations.of(c).x`.

Hors top 15 (usage plus faible ou effort L) : Fluent `.ftl` (fort côté Rust, L : parseur + syntaxe), go-i18n
(TOML + `{{.Name}}`, M), Qt `.ts` (M), rust-i18n `_version: 2` (M), polyglot `||||` (S), typesafe-i18n
`{{a|b}}` (S), Unity String Tables (L), i18next legacy v3 `_plural` (S), Symfony `%name%` (inclus au 15).

## Annexe : fixtures exécutés

`scratchpad/methods/{i18n1,arr,nat,vue,lar,sym,java,net,ftl,ng,ngx,lingui,para,dj,rails,easy,qt,goi,rs,poly,fjs,ext,arbx}`.
Résultats clés :
- `arr` : `error: … key list has a array value` sur `status`, `check`, `translate`.
- `nat` : cible écrite `{"Welcome back": {"": "⟦fr⟧ Welcome back."}}`.
- `i18n1` (pl) : seules `item_one`/`item_other` écrites, `check` « passed (0 warnings) » ; `home.json` cible reste `{}`.
- `vue` : `"pas de pommes | {count} pommes"` (2 branches sur 3) et `link` sans `@:hi` → `check passed`.
- `lar` : `"Bienvenue, :nom"` pour `":name"` → `check passed`.
- `lingui` : msgstr ICU réécrit en `msgid_plural` + `msgstr[0..2]`.
- `rails` (pattern ajouté) : `devise.ok` écrit dans `fr.yml`, pas `devise.fr.yml`.
- `fjs` : `description` traduite (`"⟦fr⟧ greeting"`).
- `java`, `net`, `ftl`, `ng`, `qt`, `goi`, `rs`, `sym`, `easy`, `ext` : `init` → stack `generic`, `0 keys`.
- `dj`, `rails`, `arbx`, `i18n1` (cas simples) : adoptés correctement.
