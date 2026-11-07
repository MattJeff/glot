# Couverture glot : mobile / desktop (2026-11-07)

Popularité relevée le 2026-11-07. npm : `api.npmjs.org/downloads/point/last-week` (semaine du 31/10 au 06/11/2026). pub.dev : `/api/packages/<p>/score` (téléchargements sur 30 jours, likes). GitHub : étoiles via l'API.
Les formats de plateforme (Xcode, Android, .NET, Qt, Unity) n'ont pas de paquet à compter. Leur ordre de grandeur vient de docs/research/i18n-methods.md.

Fixtures testées : `scratchpad/mobile/{android,cmp,moko,ios-xc,ios-lp,flutter,easy,slang,rn-i18next,expo,electron,maui,qt,unity}`.
Sur chacune, on a lancé `glot init --yes --provider mock`, puis `translate --provider mock` et `check`, et vérifié le diff git. On a ensuite injecté des erreurs de placeholder.

| Framework | Popularité (source, date) | Format | Avant | Après | Reste à faire |
|---|---|---|---|---|---|
| iOS/macOS String Catalog | défaut Xcode 15+ | `.xcstrings` JSON | OK : détection, round-trip, variations plural, printf | inchangé | substitutions `%#@x@` vérifiées comme du texte |
| iOS `.strings` / `.stringsdict` / `InfoPlist.strings` | legacy massif | `"k" = "v";`, plist | OK : UTF-16, multi-tables, `.stringsdict` créé pour la cible | inchangé | — |
| Android `strings.xml` | toute app Android | XML `<string>/<plurals>/<string-array>` | OK : `values-pt-rBR`, CDATA, `\'`, `translatable=false`, `%1$s`→`%1$d` détecté | inchangé | `arrays.xml`/`plurals.xml` séparés non détectés (motif à ajouter à la main) |
| Compose Multiplatform resources | 19 468 ★ (JetBrains/compose-multiplatform) | `composeResources/values-xx/strings.xml` | **non détecté** (repli sur `locales/{locale}.json`) | **détecté** (`compose-multiplatform`), lu et écrit comme Android | — |
| moko-resources | 1 353 ★ | `moko-resources/base/strings.xml`, `plurals.xml` avec `<plural>` | **non détecté**, `base` inconnu, `<plural>` ignoré | **détecté** ; `base` ⇄ source (`config::android_values`) ; `<plural>` lu et réécrit avec sa balise | une nouvelle clé plurielle va dans `strings.xml` |
| Flutter gen-l10n ARB | intl : 13 564 249 dl/30 j, 6 113 likes (pub.dev) | `.arb` ICU | OK | inchangé | — |
| easy_localization | 342 537 dl/30 j, 3 843 likes | JSON, `{}`/`{name}`, pluriels en sous-objet | OK (détection, pluriels, `{}` validé) | inchangé | ordre alphabétique des clés ajoutées ; message `%1$…` pour `{}` |
| slang | 392 865 dl/30 j, 794 likes (slang_flutter 362 213) | `strings.i18n.json` (base sans locale) + `strings_xx.i18n.json`, `$name`/`${name}` | **source erronée** (`fr`, base ignorée), `$name` non validé | base = `base_locale` (`slang.yaml`/`build.yaml`, sinon `en`) via `unsuffixed_source` ; `$name` validé (printf) | slang YAML/CSV non détectés ; liens `@:key` en texte |
| React Native + i18next (+ react-native-localize) | i18next 34,8 M/sem., react-native-localize 809 946/sem. | JSON i18next | OK (détection `react-i18next`) | inchangé | — |
| Expo + i18n-js (expo-localization) | expo-localization 4 969 391/sem., i18n-js 805 711/sem. | JSON `%{x}`, sous-objets `zero/one/other` | OK (syntaxe `rails`, erreurs `%{nom}` détectées) | inchangé | JSON compact sur une ligne réindenté |
| Lingui (RN) | @lingui/core 2 554 426/sem. | PO | OK (périmètre JS) | inchangé | — |
| Electron + i18next | electron 12,7 M/sem., i18next-fs-backend 1,41 M/sem. | JSON i18next | OK | inchangé | — |
| .NET MAUI/WPF (resx) | Microsoft.Extensions.Localization 615 M dl (NuGet, cf. i18n-methods) | `.resx` XML, `{0}` | OK (source sans suffixe, `{0}`→`{1}` détecté) | inchangé | messages en `%0$…` au lieu de `{0}` |
| Qt Linguist | plateforme (qttools 234 ★, miroir) | `.ts` XML, `%1`, `%n`, `<numerusform>` | **absent** : `.ts` lu comme TypeScript, `%1` non validé | **nouveau format `qt`** (`formats/qt.rs`) : détection `<app>_{locale}.ts`, source implicite (`sourcelanguage`), contexte + `comment` dans la clé, notes (`extracomment`, `location`), pluriels numerus, `unfinished` ⇒ à revoir, écriture en place ; `%1`/`%L1`/`%n` validés | pas d'extraction C++/QML (utiliser `lupdate`) ; warning `many` pour fr alors que Qt n'a que 2 formes |
| Unity Localization | pas de compteur public | String Tables `.asset` (YAML Unity), export CSV/XLIFF | absent | documenté : passer par l'export XLIFF | lecteur `.asset` / CSV (valeur faible) |

## Changements

- `crates/core/src/formats/qt.rs` (nouveau), `Format::Qt` (`catalog.rs`), branchement dans `formats/mod.rs` (`.ts` XML ⇒ Qt sous `auto`), `po::default_categories`.
- `formats/android.rs` : balise `<plural>` (moko) ; la balise fermante suit la balise ouvrante.
- `config.rs` : `moko-resources/{locale}/` et `MR/{locale}/` ⇒ `base` pour la source ; base slang sans suffixe (`unsuffixed_source`).
- `validate.rs` (printf) : Qt `%1`..`%99`, `%L1`, `%n` ; Dart/slang `$name`.
- `cli/src/init_native.rs` : Compose MP, moko, Qt, base slang ; `init.rs` écrit `format = "qt"`.
- Tests : `formats/mobile_tests.rs` (aller-retour Qt octet pour octet, édition/ajout/suppression, nouveau fichier, `unfinished`, moko `<plural>`, placeholders Qt/slang, chemins moko/slang), `init::native::tests::detects_mobile_desktop_stacks`.
- Docs : `web/content/docs/frameworks.md`, section « Mobile and desktop catalogs ».
