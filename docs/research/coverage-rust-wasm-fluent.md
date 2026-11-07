# Couverture glot : Rust / WebAssembly / Fluent

Date : 2026-11-07. Popularité : API crates.io (`recent_downloads` = 90 derniers jours, total entre parenthèses), relevée le 2026-11-07.

| Framework | Popularité (crates.io) | Format / syntaxe | Statut avant | Statut après | Reste à faire |
|---|---|---|---|---|---|
| Fluent (fluent-bundle / fluent) | 8,56 M / 90 j (23,6 M) ; `fluent` 7,08 M (17,5 M) | `.ftl` : `id = …`, `-term`, attributs `.attr`, `{ $var }`, `{ NUMBER($n) }`, sélecteurs `{ $n -> [one] … *[other] … }` | `generic`, 0 clé | géré via fichiers : `Format::Fluent` (lecture et écriture par patch, commentaires, ordre et octets inchangés gardés), `Syntax::Fluent` (variables, références, défaut `*[…]`, catégories CLDR) | Échappement `{"["}` des lignes de traduction qui commencent par `[`, `*` ou `.` ; un nouveau fichier cible ne reprend ni l'ordre ni les commentaires de la source |
| i18n-embed / cargo-i18n | 4,73 M (9,3 M) | Fluent `i18n/{locale}/{crate}.ftl`, `i18n.toml` | `generic` | détecté (`i18n-embed`), source = `fallback_language` | — |
| rust-i18n | 1,73 M (4,6 M) | YAML `_version: 1` (un fichier par locale) ou `_version: 2` (toutes les locales dans un fichier), `%{name}` | v1 lu comme du YAML Rails, `_version` traduit comme une clé ; v2 illisible ; non détecté | détecté (`i18n!("dir")`) ; v1 garde `_version` ; v2 lit et patche seulement les feuilles de la locale, et ajoute les nouvelles sous leur clé ; syntaxe `rails` | TOML `_version: 2` ; JSON v2 |
| gettext-rs | 710 k (5,2 M) | `.po` | géré (détection gettext) | inchangé : passe par la détection gettext | — |
| fluent-templates | 277 k (1,3 M) | `locales/{locale}/*.ftl` | `generic` | détecté, un motif par fichier, source = `fallback_language` de `static_loader!` | — |
| `tr` (gettext-like) | 226 k (1,8 M) | `tr!("…")` + `.po` | non | usages lus par `glot extract` (`tr!`) | — |
| leptos_i18n | 53 k (238 k) | `locales/{locale}.json` (ou `.yaml`, `{locale}/{ns}.json`), `{{ var }}`, `_one` / `_other` | `generic` si le dossier n'est pas `locales/` standard ; aucune lecture de `[package.metadata.leptos-i18n]` | détecté par la metadata (`default`, `locales`, `locales-dir`, `namespaces`) ou par les fichiers ; syntaxe `i18next` ; `t!(i18n, a.b)` lu par extract | `.json5`, pluriels par plages |
| dioxus-i18n | 30 k (83 k) | Fluent `locales/{locale}.ftl` | `generic` | détecté (`dioxus-i18n`), clés `t!("…")` et texte `rsx!` lus par extract | — |
| Yew (yew-i18n) | 447 (11 k) | pas de standard | — | texte `html!` lu par extract ; catalogue détecté selon la crate utilisée | — |
| Tauri | n/a (front JS) | catalogue du front | via la détection JS | inchangé (la détection JS passe avant) | — |
| Tables Rust maison (cliponit) | 1 cas réel : 6 + 1 tables, 2 231 clés FR/EN | `static DICT: &[(&str, &str, &str)] = &[("clé", "fr", "en"), …]`, `{n}` remplacé par `str::replace` | `generic`, 0 clé | `Format::RustDict` : colonnes nommées par le commentaire `// (clé, fr, en)` ou par le motif `\|(_, fr, en)\|` ; le patch remplace seulement le littéral modifié ; une nouvelle clé devient un tuple ajouté à la fin ; un doublon (code mort) n'est pas touché ; `Syntax::Plain` (`{n}` est le seul élément de syntaxe) ; détection multi-fichiers plus l'extension WebExtension voisine | Une nouvelle locale demande une nouvelle colonne (refusée avec un message clair) ; 500 paires en ligne `tr("fr", "en")` sont signalées, pas migrées |
| Extraction `view!` / `html!` / `rsx!` | — | littéraux de texte et attributs visibles | aucune (seuls les `.rs` hors JS) | `crates/extract/src/rust.rs` : usages (`t`, `tn`, `tc`, `tl`, `t!`, `fl!`, lookups locales `fn lt(key: &str)`), préfixes dynamiques `t(&format!("a{n}"))`, texte en dur ; `glot init` ajoute `src/**/*.rs` | Réécriture automatique (`i18nize`) pour Rust |

## Cas cliponit (copie de `~/Desktop/saasclip` dans le scratchpad, sans `.env`)

- `glot init --yes` donne : stack `rust (string tables)`, source `fr`, cible `en`, syntaxe `plain`, 7 fichiers `.rs` plus `extension/_locales/{locale}/messages.json`.
- `glot status` : 2 231 clés, `en` complet à 100 %. `glot check` passe avec 111 avertissements, tous des `untranslated` (noms de marque, mots identiques en FR et en EN).
- Avant `Syntax::Plain`, l'ICU produisait 5 fausses erreurs (`{{sujet}}` et `<clé>` sont du texte dans cette app).
- Une clé ajoutée avec `""` en `en`, puis `glot translate --provider mock` : la diff fait 1 ligne. Trois clés de `i18n.rs` sont en double avec des valeurs différentes (`xp_none`…). La 2e occurrence est du code mort : glot la laisse telle quelle.
- `glot extract` remonte 200 textes en dur dans `view!` (l'admin est en FR seulement, des `aria-label` « Supprimer » sont en dur dans l'éditeur), 0 clé manquante hors `narrated` (FR volontairement vide) et 14 fichiers à paires en ligne `tr("…", "…")`.
