# Field test : natif / mobile / autres (2026-10-18)

Les projets non-JS du fondateur ont été testés sur des copies (rsync sans `.git`, builds, secrets, médias),
avec le provider `mock` uniquement. Les copies ont été supprimées ensuite. Les noms des projets sont anonymisés, et ce document
ne contient aucun contenu de catalogue (seulement des structures et des comptes).

## Inventaire

| Projet | Stack | État i18n | `glot init` | Round-trip* |
|---|---|---|---|---|
| ios-a | SwiftUI, 204 `.swift` | 5 tables `.strings` (Auth/Errors/Localizable/Main/Onboarding) × 13 langues, clés pointées via un enum généré (`Text(L10n.…)` ×534) | `ios`, 5 motifs, 12 cibles ✓ | 65/65 ✓ |
| ios-b | SwiftUI, 35 `.swift` | 1 `Localizable.strings` × 13 langues, clés en langage naturel, helper `L10n.tr("…")` | `ios` ✓ | 13/13 ✓ (après correctif doublons) |
| osx-legacy | app macOS ObjC/C++ (2005) | `English.lproj`/`French.lproj`, `.strings` **UTF-16 LE + CRLF** | avant : `generic` (rien trouvé) ; après : message « renomme en en.lproj/fr.lproj » | 4/4 ✓ (UTF-16 préservé) |
| po-big | grosse app C/Python (gettext) | 49 `locale/po/<lang>.po` sans `en.po` ni `.pot`, 39 292 msgids, Python `{}`/`{:d}` + printf C | avant : source `ab` et 0 clé ; après : source `en`, 39 292 clés | 49/49 ✓ (80 Mo) |
| rust-leptos | Leptos 0.8 CSR (Trunk) | i18n faite main : `static DICT: &[(&str,&str,&str)]` (~430 entrées fr/en dans un `.rs`), `t("key")` ×365 ; blog en `*.en.html`/`*.fr.html` | `generic` (aucun catalogue) | n/a |
| tauri | Tauri 2 + React | aucune i18n | `react-i18next` (502 chaînes en dur, géré par l'agent JS) | n/a |
| py-a, py-b | FastAPI (backends) | aucune ; 29 `HTTPException(detail="…")` et 3 templates HTML | `generic` | n/a |
| — | Android, Flutter, xcstrings | **absents** du Mac (ni `strings.xml`, ni `.arb`, ni `.xcstrings`) | couverts seulement par les fixtures | — |

\* Round-trip : `GLOT_ROUNDTRIP_DIR=<dir> cargo test -p glot-core roundtrip_dir -- --ignored`. Chaque catalogue natif est
relu puis réécrit sans changement, et le résultat doit être identique octet pour octet, encodage compris. On réécrit ensuite une valeur
modifiée (guillemets, antislash) et on vérifie qu'on la relit à l'identique.

Un `glot translate` (mock, 3 langues) sur ios-a n'a produit **que des ajouts** (217 lignes). Chaque clé a été rangée dans la table
qui la contient en source. Aucune autre ligne n'a bougé.

## Bugs trouvés et corrigés

| # | Symptôme réel | Cause | Correctif |
|---|---|---|---|
| 1 | `.strings` UTF-16 (Xcode ancien) : `glot` plante (« stream did not contain valid UTF-8 ») | `read_to_string` partout | `formats::{decode,encode,write_path}` : UTF-8, UTF-8 BOM et UTF-16 LE/BE avec BOM sont lus, puis réécrits **dans le même encodage** (`catalog.rs` passe par `write_path`) |
| 2 | Fichier CRLF : les entrées ajoutées sont en LF (fins de ligne mélangées) | append en `\n` | `write_path` suit le CRLF du fichier quand il est homogène |
| 3 | Fausse erreur `placeholder_missing %2019$` sur `friend\u{2019}s` | en syntaxe printf, `{2019}` était lu comme un argument ICU | `\u{…}` est décodé avant le parse printf. Le texte n'est plus pris pour un placeholder |
| 4 | `\u{2019}` à backslash simple → texte corrompu (`9}s`) | `unescape` consommait 4 caractères quelconques | `\U`/`\u` ne sont décodés que suivis de 4 chiffres hexadécimaux. Sinon le texte est gardé tel quel et signalé (#6) |
| 5 | Clé dupliquée dans un `.strings` (une langue d'ios-b) : la réécriture **sans changement** modifiait le fichier | la 1re occurrence recevait la valeur de la dernière | une occurrence masquée n'est plus touchée (Foundation retient la dernière, comme glot à la lecture) |
| 6 | 41 clés poubelles dans ios-b (`— \(L10n.tr(`, `#\(entry.rank)`, `Continue \u{2192}`) invisibles | aucune vérification côté source | nouveau `validate::lint_source` → code `suspicious_source`, **une fois par clé** (locale source) : interpolation Swift `\(`, `L10n.tr(`, `NSLocalizedString(`, `String(localized:`, échappement Swift `\u{…}` (le message propose le caractère et `\UXXXX`). `glot status` affiche `! N suspicious source keys` |
| 7 | 3 087 avertissements `untranslated` sur ios-b (11 langues = copie de l'anglais), illisibles | pas d'agrégat | `LocaleStatus.untranslated`. `glot status` affiche une ligne `! Probably not translated: … (ar 276/411, …)` quand plus de 50 % des valeurs sont identiques à la source (minimum 10), avec la commande de retraduction |
| 8 | po-big : `en_GB` signalé comme « non traduit » (35 559 avertissements) | une variante de la langue source recopie légitimement la source | pas d'`untranslated` quand la cible a la même langue que la source (`en_GB`/`en`, `fr_CA`/`fr`) |
| 9 | gettext sans `en.po` : source `ab` choisie par ordre alphabétique, 0 clé | les msgids des cibles n'étaient pas utilisés comme source | `init` ajoute `en` pour gettext. `load_meta` construit la source à partir des msgids des cibles quand aucun fichier source n'existe (rien n'est écrit) |
| 10 | po-big : 2 001 fausses `syntax_error` (`{:d}`, `{}`, `{name!r}`, `{` littéral) | en printf, toute accolade passait par le parseur ICU | `printf_braces` : `{n, plural, …}` reste ICU. Les champs `str.format` deviennent des placeholders, avec conversion et spec comme format, donc `{:d}`→`{:s}` est détecté. Toute autre accolade est du texte |
| 11 | po-big : `tag <NONE> is never closed` (texte littéral `<NONE>`) | équilibre des balises exigé en printf | en printf, les balises sont comparées (`tag_mismatch`) mais pas exigées équilibrées |
| 12 | Plusieurs tables `.strings` : `glot add` écrivait dans `Auth.strings` ou `InfoPlist.strings` (1er motif alphabétique) | ordre des motifs | `init` met `Localizable.*` en premier et `InfoPlist.*` en dernier |
| 13 | `English.lproj`/`French.lproj` ignorés en silence | `is_locale("English")` = faux | `init` affiche « renomme en en.lproj / fr.lproj dans Xcode » |

Sur po-big, après correctifs, il reste 341 erreurs sur 1,8 M d'entrées. Ce sont de **vrais bugs de traduction** : `{:d}` devenu `{:s}`
(ValueError en Python), `!r` devenu `! R`, `%s`/`%d` intervertis sans positions, balises `<memory>` perdues. Une seule erreur mal attribuée
demeure : pour `{}`, le message affiche `%0$…`, alors que le champ est automatique (voir les limites).

Tests de non-régression, avec des fixtures inline anonymisées :
- `glot-core` : `formats::tests::native_field_regressions` (UTF-16, CRLF, BOM, `\U`/`\u{}`, doublons) et `printf_field_regressions` (Swift, Python, accolades, `<NONE>`, `lint_source`), plus `roundtrip_dir` (`#[ignore]`, pour les projets réels) ;
- `glot-llm` : `mobile_formats::gettext_without_source_file` ;
- `glot-cli` : `init::native::tests::detects_each_stack`, avec un cas multi-tables (Localizable en premier, InfoPlist en dernier, Base ignoré) et un cas gettext sans `en`.

## Limites restantes (non corrigées)

- **Tables `.strings` fusionnées** : glot met toutes les tables dans un seul espace de clés. Une même clé présente dans deux tables avec des valeurs
  différentes serait écrasée sans avertissement. Sur ios-a, 2 clés sont dans ce cas, avec des valeurs identiques. Il faudrait un warning à la lecture.
- **Doublons dans un fichier** : ils sont désormais préservés, mais rien ne les signale encore (Xcode avertit).
- **Python `{}`** : le message parle de `%0$…`. Le nommage `ph()` ne distingue pas un champ automatique Python d'une position printf.
- **Perf** : `glot status` et `glot check` prennent environ 60 s en debug sur 49 × 39 k entrées PO. C'est acceptable, mais la validation est refaite par `status`.
- **Noms `.lproj` hérités** : on les signale seulement. Un alias de locale dans `glot.toml` permettrait de ne pas renommer.
- `functions` du `glot.toml` iOS reste la liste JS (`t`, `useTranslations`…), sans effet tant que l'extraction Swift n'existe pas.
- `status` affiche « Up to date. » juste après les avertissements agrégés. C'est exact pour le plan, mais peut prêter à confusion.

## Propositions (classées valeur/effort)

1. **Extraction Swift/SwiftUI (`glot extract` + `i18nize` iOS)**, valeur haute, effort moyen (environ 3 à 5 jours).
   Constat sur ios-b : 366 littéraux dans `Text/Button/Label/navigationTitle/L10n.tr`. 226 sont dans le catalogue, 102 n'y sont pas
   (non localisés) et 38 contiennent `\(…)`. Ces 38 sont **à l'origine des clés poubelles** : le script maison a recopié `\(x)` au lieu de produire la clé
   SwiftUI.
   - Lecture : un lexer de littéraux Swift (`"…"`, `"""…"""`, `#"…"#`, interpolation `\(…)` imbriquée) suffit, sans parser complet.
     Les appels à cibler : `Text("…")`, `Button("…")`, `Label("…", systemImage:)`, `.navigationTitle("…")`, `Toggle`, `Section`, `TextField`,
     `NSLocalizedString("…", comment:)` (ce comment devient une note), `String(localized:)`, plus des `functions` configurables (`L10n.tr`). À ignorer :
     `Text(verbatim:)`, `Text(someVar)`, `print`, `Logger`.
   - Clé SwiftUI exacte : `\(x)` devient `%@`, `\(n)` (Int) devient `%lld`, `\(d)` (Double) devient `%lf`. Le type est inconnu sans compilateur : on prend `%@` par défaut
     et une heuristique de nom (`count`, `n`, `index` donnent `%lld`). C'est le point délicat. `xcodebuild -exportLocalizations` reste la
     vérité terrain : proposer `glot extract --xcode` qui l'appelle quand Xcode est présent.
   - i18nize iOS : il n'y a **rien à réécrire** pour SwiftUI, puisque `Text("Hello")` est déjà localisable. Il suffit d'ajouter la clé au catalogue, ce qui est bien
     moins risqué qu'en JS. Ne réécrire que `Text(stringVar)`, qui devient `Text(LocalizedStringKey(stringVar))`, et les `String` passées à des API UIKit, qui deviennent `String(localized:)`.
   - Migration `.strings` vers `.xcstrings` (déjà supporté en lecture et écriture) : c'est Xcode qui la fait, glot ne doit pas la réimplémenter.
2. **Format « dictionnaire Rust »**, valeur moyenne pour ce Mac (le front Leptos), effort faible (1 jour).
   Un tableau statique `&[(key, fr, en)]` ou un `match key { "a" => "…" }` dans un `.rs`, patché en place comme `formats/js.rs`.
   Alternative plus simple, à recommander d'abord : déplacer vers `locales/{fr,en}.json` + `include_str!` + `serde_json`. glot gère déjà ce format,
   et `i18nize` pourrait générer ce squelette.
3. **Extraction Leptos/Yew `view!`**, valeur moyenne, effort élevé (environ 1 semaine). Il faut `syn` et du parsing de tokens, car les macros ne sont pas du Rust
   standard : les nœuds texte `"…"` enfants d'éléments et les attributs `placeholder/title/aria-label/alt` (environ 60 occurrences sur le front testé). La réécriture
   `"Text"` → `{t("key")}` est mécanique dans `view!` (Leptos) comme dans `html!` (Yew), mais sans runtime i18n standard (leptos_i18n,
   fluent, maison). À ne faire qu'après le point 2.
4. **Python** : gettext est déjà bien couvert après correctifs, avec `python-brace-format` et `%(name)s`. Les backends FastAPI testés n'ont pas de
   texte UI (erreurs d'API), donc la valeur est faible. Pour Django/Flask, `makemessages`/`pybabel extract` font l'extraction. glot devrait juste les appeler
   (`glot extract --gettext`), avec un effort faible.
5. Petits gains : un warning sur les doublons et collisions de tables (0,5 jour), un alias de locale `English => en` (0,5 jour), et
   `ph()` qui affiche `{}` / `{0}` pour un champ Python (0,5 jour).
