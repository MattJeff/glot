# Couverture back-end — frameworks serveur × glot

> 2026-11-07. Périmètre : Rails, Django, Flask-Babel, Laravel, Symfony, Spring, Java ResourceBundle, ASP.NET (resx),
> Go (go-i18n, gotext), Elixir/Phoenix, Express/Nest, Python/PHP gettext, WordPress, Strapi/Directus.
> Méthode : fixtures réalistes dans `scratchpad/backend/<framework>/` (hors repo), `target/integ/debug/glot init --yes
> --provider mock`, `translate --provider mock --yes`, `check`, puis `git diff` des catalogues. Tests aller-retour
> dans `crates/core/src/formats/backend_tests.rs`.

## Popularité (relevée le 2026-11-07)

| Écosystème | Mesure | Source |
|---|---|---|
| WordPress | ~40 % des sites web, ~59 % des sites à CMS connu (oct. 2026) | W3Techs via colorlib.com/wp/cms-market-share (W3Techs non consulté directement) |
| Rails I18n | gem `i18n` 1 475 249 384 dl total ; `rails-i18n` 146 M ; `gettext_i18n_rails` 45,6 M ; `i18n-tasks` 35,2 M | rubygems.org API |
| Python | `babel` 101,3 M dl/30 j (30,1 M/sem.) ; `django` 41,1 M/30 j ; `flask-babel` 5,96 M/30 j ; `polib` 4,86 M/30 j ; `django-modeltranslation` 0,76 M/30 j | pypistats.org |
| PHP | `symfony/translation` 28,4 M dl/mois (975 M total) ; `laravel/framework` 22,5 M/mois (615 M total) ; `gettext/gettext` 1,16 M/mois | packagist.org stats |
| .NET | `Microsoft.Extensions.Localization` 631 M dl total | NuGet search API |
| Java / Spring | `spring-boot` 81 786 ⭐ ; `messages.properties` = convention de plateforme (pas de paquet à compter) | GitHub API |
| Elixir | `gettext` (hex) 166 M dl total, 4,1 M récents, 392 k/sem. | hex.pm API |
| Node | `i18next` 34,8 M/sem. ; `i18next-fs-backend` 1,41 M ; `nestjs-i18n` 615 k ; `i18n` (mashpie) 596 k ; `i18next-http-middleware` 395 k ; `node-gettext` 80 k ; `gettext-parser` 1,53 M | api.npmjs.org, semaine 2026-10-31 → 11-06 |
| Go | `nicksnyder/go-i18n` 3 552 ⭐ (v2.7.0 du 2026-10-27) ; `golang/text` 810 ⭐ (gotext marqué expérimental) | GitHub API, proxy.golang.org |
| CMS headless | `@strapi/strapi` 271 k/sem., 73 442 ⭐ ; `@directus/sdk` 190 k/sem., 38 680 ⭐ | npm, GitHub |

Ordre de valeur retenu : WordPress > Symfony ≈ Laravel ≈ Rails ≈ Django/Babel > Spring/.NET > Express/Nest > Phoenix > Go > Strapi/Directus.

## Matrice

Statut : **OK** (détecté, lu/écrit à l'identique, syntaxe validée), **partiel**, **absent**.

| Framework | Format | Avant (constaté) | Après | Reste à faire |
|---|---|---|---|---|
| Rails | YAML `config/locales/**/*.{l}.yml`, `%{x}`, `one/other` | OK (pattern par fichier, `devise.fr.yml` créé en miroir, `few/many` ajoutés) | OK | strftime `date.formats` traduit comme du texte |
| Django | PO `locale/{l}/LC_MESSAGES/django.po`, `%(x)s`, `{x}` | OK (msgctxt, pluriels, en-têtes intacts) | OK | `.mo` à compiler par l'utilisateur (`compilemessages`) |
| Flask-Babel / Babel | PO + `messages.pot` à la racine | partiel : le `.pot` ignoré, un msgid présent seulement dans le `.pot` n'était jamais traduit | OK : `.pot` = source quand il n'y a pas de `.po` source (dossier ou parents), et modèle des nouvelles entrées | `pybabel compile` |
| Elixir / Phoenix | PO `priv/gettext/{l}/LC_MESSAGES/*.po` + `*.pot`, `%{x}` | partiel : idem (sans `mix gettext.extract --merge`, nouveaux msgids invisibles ; nouvelles entrées sans commentaires ni `msgid_plural` réel) | OK : `priv/gettext/default.pot` source + modèle (`#.`, `#:`, `msgid_plural` copiés) | — |
| PHP gettext | PO `locale/{l}/LC_MESSAGES/*.po` | OK | OK | `.mo` (msgfmt) |
| WordPress | PO/POT `languages/{domain}-{l}.po` | traité par l'agent CMS (détection `wordpress-plugin/theme`, `.pot` source) | idem | `.mo` / `.l10n.php` (`wp i18n make-mo/make-php`) |
| Laravel | `lang/{l}.json` + `lang/{l}/*.php`, `:name`, `a\|b`, `{0}…\|[2,*]` | OK (détection, PHP patché, syntaxe `laravel`) | OK | Symfony-like `.php` hors Laravel non géré |
| Symfony | YAML/XLIFF `translations/{domain}.{l}.*`, `+intl-icu` | partiel : syntaxe `icu` → `{{ value }}` des validators **rejeté** comme ICU invalide, `%name%` et intervalles `]1,Inf[` non vérifiés | OK : syntaxe `symfony` (ICU pour les messages avec plural/select, sinon `%x%`, `{{ x }}`, `{x}`, formes `\|` avec intervalles conservés, nombre de formes par langue) | `.php`/`.json`/`.csv` Symfony |
| Spring | `.properties`, MessageFormat `{0}` | partiel : syntaxe `icu` → `L'utilisateur {0}` accepté alors que Java affiche `Lutilisateur {0}` | OK : syntaxe `java` (quotes Java, `''`, `{0,choice}` ; erreur explicite sur `'` seul quand il y a des arguments ; message sans `{` = texte brut comme Spring par défaut) | `alwaysUseMessageFormat=true` non pris en compte |
| Java ResourceBundle | `.properties` | idem Spring (stack `java`) | OK (`java`) | — |
| ASP.NET / .NET | `.resx` + `.{culture}.resx` | OK (`{0:C2}`, `{{`, `<comment>` → note, fichier cible créé avec resheaders) | OK | — |
| Go go-i18n v2 | TOML/JSON/YAML `active.{l}.*`, `{{.Name}}` | absent : stack `generic`, `locales/{l}.json`, syntaxe ICU (format TOML ajouté en parallèle par l'agent CMS) | OK en TOML : détection `go-i18n` (`go.mod`), syntaxe `template`, `few/many` complétés | JSON/YAML : un `description` à côté des formes casse le repli pluriel et est traduit |
| Go gotext | JSON `messages[]` | absent | absent (documenté) | format `gotext.json` (faible usage, outil expérimental) |
| Express + i18next | JSON i18next | OK | OK | — |
| Express + `i18n` (mashpie) | `locales/{l}.json`, `%s`, `{{x}}` | partiel : stack `generic`, ICU → `{{name}}` **rejeté** | OK : stack `i18n (node)`, syntaxe `printf` | mustache `{{x}}` non vérifié |
| NestJS (nestjs-i18n) | `src/i18n/{l}/*.json`, `{x}`, sous-objets pluriels | partiel : stack `generic`, un seul fichier retenu | OK : stack `nestjs-i18n`, un pattern par fichier de namespace | `$t()` vérifié comme texte |
| Strapi / Directus | contenu en base | — | documenté (`glot db` ou export) | connecteur dédié |

## Bugs trouvés hors périmètre direct

- `po.rs` (nouvelle entrée sans modèle) : `msgid_plural` répète le singulier (`"%d comment"`), corrigé quand un `.pot`
  est trouvé ; sans `.pot` la limite reste (commentaire `ponytail` existant).

## Fichiers

- `crates/core/src/validate/backend.rs` (nouveau) : syntaxes `symfony`, `java` + tests.
- `crates/core/src/validate.rs` : variantes `Symfony`, `Java` (insertions), intervalles `]a,b[` dans `laravel_range`,
  libellé « Symfony plural ».
- `crates/llm/src/translate.rs` : règles de prompt `symfony`, `java`.
- `crates/core/src/config.rs` : `gettext_pot` + `.pot` source pour `{dir}/{locale}/LC_MESSAGES/{domain}.po`.
- `crates/core/src/formats/mod.rs` : `pot_template` pour les dossiers `LC_MESSAGES`.
- `crates/core/src/formats/backend_tests.rs` (nouveau) : aller-retour + diff d'une ligne (Symfony YAML/XLIFF, Spring,
  resx, Laravel, Phoenix, Rails devise, go-i18n), quoting YAML, placeholders par framework, `.pot` Phoenix/Babel.
- `crates/cli/src/init_backend.rs` (nouveau) : détection go-i18n, nestjs-i18n, `i18n` (node) + tests.
- `crates/cli/src/init.rs`, `init_native.rs` : branchement, syntaxes `symfony` / `java`.
- `web/content/docs/frameworks.md` : section « Back-end frameworks ».
