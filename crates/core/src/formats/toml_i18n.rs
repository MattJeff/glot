//! TOML i18n catalogs: Hugo `i18n/fr.toml` and go-i18n v2 `active.fr.toml`.
//! `id = "text"` is a message; so is a table `[id]` holding CLDR categories (`one`/`other`…) plus
//! go-i18n metadata (`description` becomes a translator note, `hash` and the delimiters are kept).
//! A table with `other` alone is a plain message; with more categories, an ICU plural
//! `{_count, plural, one {…} other {…}}` re-split into the target locale's categories on write.
//! Other tables nest keys (`[nav] home = …` => `nav.home`). Writing patches the document through
//! `toml_edit`: comments, order and untouched values are kept byte for byte.
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use super::{Parsed, folded_choice, icu_choice};
use crate::catalog::Catalog;

const CATS: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
const META: [&str; 4] = ["description", "hash", "leftdelim", "rightdelim"];
const VAR: &str = "_count";

/// Categories of a message table, `None` when `t` is not one.
fn message(t: &dyn TableLike) -> Option<Vec<(String, String)>> {
    let mut forms = vec![];
    for (k, v) in t.iter() {
        let kl = k.to_ascii_lowercase();
        if CATS.contains(&kl.as_str()) {
            forms.push((kl, v.as_str()?.to_string()));
        } else if !META.contains(&kl.as_str()) {
            return None;
        }
    }
    forms.iter().any(|(k, _)| k == "other").then_some(forms)
}

fn text_of(forms: &[(String, String)]) -> String {
    match forms {
        [(_, v)] => v.clone(),
        _ => icu_choice(VAR, "plural", forms),
    }
}

fn walk(t: &dyn TableLike, prefix: &str, p: &mut Parsed) {
    for (k, item) in t.iter() {
        let key = format!("{prefix}{k}");
        if let Some(s) = item.as_str() {
            p.entries.insert(key, s.to_string());
        } else if let Some(sub) = item.as_table_like() {
            match message(sub) {
                Some(forms) => {
                    if let Some(d) = sub.get("description").and_then(Item::as_str) {
                        p.notes.insert(key.clone(), d.to_string());
                    }
                    p.entries.insert(key, text_of(&forms));
                }
                None => walk(sub, &format!("{key}."), p),
            }
        }
    }
}

pub fn parse(text: &str) -> Result<Parsed, String> {
    let doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let mut p = Parsed::default();
    walk(doc.as_table(), "", &mut p);
    Ok(p)
}

/// Replace a string value, keeping its decoration (spacing, trailing comment).
fn set_str(item: &mut Item, s: &str) {
    if item.as_str() == Some(s) {
        return;
    }
    let decor = item.as_value().map(|v| v.decor().clone());
    *item = Item::Value(Value::from(s));
    if let (Some(d), Some(v)) = (decor, item.as_value_mut()) {
        *v.decor_mut() = d;
    }
}

/// Write `value` into the message table `t` for `locale`'s plural categories.
fn set_forms(t: &mut dyn TableLike, value: &str, locale: &str) {
    let forms: Vec<(String, String)> = match folded_choice(value).filter(|c| c.kind == "plural") {
        Some(c) => crate::plural::categories(locale)
            .iter()
            .map(|cat| {
                let cat = cat.as_str();
                let text = c.form(cat).or_else(|| c.form("other")).unwrap_or_default();
                (cat.to_string(), text.to_string())
            })
            .collect(),
        None => vec![("other".into(), value.to_string())],
    };
    let stale: Vec<String> = t
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| {
            CATS.contains(&k.to_ascii_lowercase().as_str()) && !forms.iter().any(|(c, _)| c == k)
        })
        .collect();
    for k in stale {
        t.remove(&k);
    }
    for (c, v) in forms {
        match t.get_mut(&c) {
            Some(item) => set_str(item, &v),
            None => {
                t.insert(&c, Item::Value(Value::from(v)));
            }
        }
    }
}

/// Patch existing messages under `t`; `done` collects the keys found.
fn patch(
    t: &mut dyn TableLike,
    prefix: &str,
    cat: &Catalog,
    done: &mut std::collections::BTreeSet<String>,
) {
    let keys: Vec<String> = t.iter().map(|(k, _)| k.to_string()).collect();
    for k in keys {
        let key = format!("{prefix}{k}");
        let item = t.get_mut(&k).expect("listed key");
        let is_message = item.as_table_like().is_some_and(|s| message(s).is_some());
        if item.is_str() || is_message {
            match cat.entries.get(&key) {
                None => {
                    t.remove(&k);
                }
                Some(v) => {
                    done.insert(key);
                    if item.is_str() && folded_choice(v).is_none() {
                        set_str(item, v);
                    } else if is_message {
                        let sub = item.as_table_like_mut().expect("message table");
                        if message(sub).map(|f| text_of(&f)).as_deref() != Some(v.as_str()) {
                            set_forms(sub, v, &cat.locale);
                        }
                    } else {
                        // A plain value that became a plural: `[id]` table.
                        let mut tab = Table::new();
                        set_forms(&mut tab, v, &cat.locale);
                        *item = Item::Table(tab);
                    }
                }
            }
        } else if let Some(sub) = item.as_table_like_mut() {
            patch(sub, &format!("{key}."), cat, done);
        }
    }
}

/// Insert `key` (dotted) under `t`, creating implicit tables; a dotted id whose parent is a
/// value stays one quoted key.
fn insert(t: &mut Table, key: &str, item: Item) {
    if let Some((head, rest)) = key.split_once('.') {
        let entry = t.entry(head).or_insert_with(|| {
            let mut s = Table::new();
            s.set_implicit(true);
            Item::Table(s)
        });
        if let Some(sub) = entry.as_table_mut() {
            return insert(sub, rest, item);
        }
    }
    t.insert(key, item);
}

pub fn write(existing: Option<&str>, cat: &Catalog) -> String {
    let mut doc: DocumentMut = existing.and_then(|t| t.parse().ok()).unwrap_or_default();
    let mut done = std::collections::BTreeSet::new();
    patch(doc.as_table_mut(), "", cat, &mut done);
    for (k, v) in cat.entries.iter().filter(|(k, _)| !done.contains(*k)) {
        // ponytail: new keys at the root (Hugo and go-i18n ids are flat); dotted ids nest in tables.
        let item = if folded_choice(v).is_some_and(|c| c.kind == "plural") {
            let mut tab = Table::new();
            set_forms(&mut tab, v, &cat.locale);
            Item::Table(tab)
        } else {
            Item::Value(Value::from(v.as_str()))
        };
        insert(doc.as_table_mut(), k, item);
    }
    doc.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUGO: &str = r#"# Navigation
home = "Home" # top bar

[readMore]
other = "Read more"

[wordCount]
description = "Words in a post"
one = "{{ .Count }} word"
other = "{{ .Count }} words"

[nav]
about = 'About us'
"#;

    fn cat(locale: &str, pairs: &[(&str, &str)]) -> Catalog {
        Catalog {
            locale: locale.into(),
            entries: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn hugo_messages_and_plurals() {
        let p = parse(HUGO).unwrap();
        assert_eq!(p.entries["home"], "Home");
        assert_eq!(p.entries["readMore"], "Read more");
        assert_eq!(
            p.entries["wordCount"],
            "{_count, plural, one {{{ .Count }} word} other {{{ .Count }} words}}"
        );
        assert_eq!(p.entries["nav.about"], "About us");
        assert_eq!(p.notes["wordCount"], "Words in a post");
        assert_eq!(p.entries.len(), 4);
    }

    #[test]
    fn round_trip_is_identical() {
        let p = parse(HUGO).unwrap();
        let c = Catalog {
            locale: "en".into(),
            entries: p.entries,
        };
        assert_eq!(write(Some(HUGO), &c), HUGO);
    }

    #[test]
    fn target_gets_its_plural_categories() {
        let p = parse(HUGO).unwrap();
        let mut c = Catalog {
            locale: "pl".into(),
            entries: p.entries,
        };
        c.entries.insert("home".into(), "Strona".into());
        c.entries.remove("nav.about");
        c.entries.insert("tags".into(), "Tagi".into());
        c.entries.insert(
            "wordCount".into(),
            "{_count, plural, one {{{ .Count }} słowo} few {{{ .Count }} słowa} other {{{ .Count }} słów}}".into(),
        );
        let out = write(Some(HUGO), &c);
        assert!(out.contains("home = \"Strona\" # top bar"), "{out}");
        assert!(out.contains("few = \"{{ .Count }} słowa\""), "{out}");
        assert!(out.contains("description = \"Words in a post\""), "{out}");
        assert!(out.contains("many = \"{{ .Count }} słów\""), "{out}");
        assert!(!out.contains("about"), "{out}");
        assert!(out.contains("tags = \"Tagi\""), "{out}");
        // Root keys stay before the first table.
        assert!(
            out.find("tags").unwrap() < out.find("[readMore]").unwrap(),
            "{out}"
        );
        let back = parse(&out).unwrap();
        assert_eq!(back.entries["home"], "Strona");
        assert_eq!(back.entries["readMore"], "Read more");
    }

    #[test]
    fn template_syntax_validates_placeholders_and_plurals() {
        use crate::validate::{IssueCode, Syntax, validate};
        let codes = |src: &str, tgt: &str, loc: &str| -> Vec<IssueCode> {
            validate(src, tgt, loc, Syntax::Template, &Default::default())
                .into_iter()
                .map(|i| i.code)
                .collect()
        };
        let ok = |src, tgt, loc| {
            let c = codes(src, tgt, loc);
            assert!(
                !c.iter().any(|c| matches!(
                    c,
                    IssueCode::PlaceholderMissing
                        | IssueCode::PlaceholderExtra
                        | IssueCode::SyntaxError
                        | IssueCode::PluralCategoryMissing
                )),
                "{src} -> {tgt}: {c:?}"
            );
        };
        ok("Hi {{ .Name }}", "Salut {{ .Name }}", "fr");
        ok(
            "From <span>{{ price | money }}</span>",
            "Dès <span>{{price|money}}</span>",
            "fr",
        );
        ok("It's {{ count }}", "C'est {{ count }}", "fr");
        let src = "{_count, plural, one {{{ count }} item} other {{{ count }} items}}";
        ok(
            src,
            "{_count, plural, one {{{ count }} article} many {{{ count }} articles} other {{{ count }} articles}}",
            "fr",
        );
        assert!(
            codes("Hi {{ .Name }}", "Salut {{ .Nom }}", "fr")
                .contains(&IssueCode::PlaceholderMissing)
        );
        assert!(
            codes(
                src,
                "{_count, plural, one {{{ count }} rzecz} other {{{ count }} rzeczy}}",
                "pl"
            )
            .contains(&IssueCode::PluralCategoryMissing)
        );
    }

    #[test]
    fn new_file_and_new_plural() {
        let c = cat(
            "de",
            &[
                ("home", "Accueil"),
                (
                    "wordCount",
                    "{_count, plural, one {{{ .Count }} mot} other {{{ .Count }} mots}}",
                ),
            ],
        );
        let out = write(None, &c);
        let back = parse(&out).unwrap();
        assert_eq!(back.entries, c.entries, "{out}");
    }
}
