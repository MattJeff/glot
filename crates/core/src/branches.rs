//! Branch coverage of ICU / MF2 messages: every combination of selector cases a user can actually see.
//!
//! For each `plural` variable: every CLDR category of the locale (with a real witness number, e.g.
//! `pl` few = 3, many = 11, other = 1.5) plus each explicit `=N`; `selectordinal` likewise; `select`:
//! each key plus `other`. The cartesian product is rendered as plain text (tags dropped, other
//! placeholders with sample values) so each branch can be measured (`measure::check`) or mounted in
//! a browser (`glot coverage --render`).
//!
//! Child module of `validate` (declared there) to walk its private AST.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::{Kind, Node, Syntax, parse, render_sample};
use crate::plural::{PluralCategory, categories, language};

/// Most combinations enumerated per message (deeply nested selects explode).
pub const MAX_BRANCHES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    /// Stable id: `count=few` / `gender=female,count==0` (the `cases`, joined).
    pub id: String,
    /// (variable, case) for each selector this branch goes through: `few`, `=0`, `female`, `other`.
    pub cases: Vec<(String, String)>,
    /// Value injected for each selector variable (witness numbers, select keys).
    pub values: BTreeMap<String, String>,
    /// Case of the innermost plural/ordinal reached (`few`, `=0`), else `select` (selects only).
    pub category: String,
    /// The message as rendered for these values.
    pub text: String,
}

/// A number of `locale` falling in `cat` (CLDR 44 cardinal rules).
pub fn witness(locale: &str, cat: PluralCategory) -> &'static str {
    use PluralCategory::*;
    let lang = language(locale);
    match (cat, lang.as_str()) {
        (Zero, _) => "0",
        (One, _) => "1",
        (Two, _) => "2",
        (Few, _) => "3",
        (Many, "cs" | "sk" | "lt") => "1.5",
        (Many, "ga") => "7",
        (Many, "cy") => "6",
        (Many, "fr" | "es" | "it" | "pt" | "ca" | "vec" | "br") => "1000000",
        (Many, _) => "11",
        (Other, "ru" | "uk" | "be" | "pl") => "1.5",
        (Other, "ar" | "ars") => "100",
        (Other, "ga" | "lt") => "11",
        (Other, "ro" | "mo" | "gd" | "mt") => "20",
        (Other, _) => "5",
    }
}

/// CLDR cardinal categories of `locale` with a witness number each (`ar`: 0, 1, 2, 3, 11, 100).
pub fn witnesses(locale: &str) -> Vec<(PluralCategory, &'static str)> {
    categories(locale)
        .into_iter()
        .map(|c| (c, witness(locale, c)))
        .collect()
}

/// Ordinal categories (`selectordinal`, MF2 `:number select=ordinal`) with witnesses, any CLDR locale (ICU4X).
pub fn ordinal_witnesses(locale: &str) -> Vec<(PluralCategory, &'static str)> {
    const N: [&str; 22] = [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "20", "21", "22",
        "23", "80", "100", "800", "1000", "0",
    ];
    crate::plural::ordinal_categories(locale)
        .into_iter()
        .filter_map(|c| {
            N.iter()
                .find(|n| crate::plural::category(locale, n, true) == Some(c))
                .map(|n| (c, *n))
        })
        .collect()
}

/// Every branch of `message` for `locale`. A message without selectors has one branch, id `""`.
pub fn enumerate(message: &str, locale: &str, syntax: Syntax) -> Result<Vec<Branch>, String> {
    let mut nodes = parse(message, syntax)?;
    // MF2 complex messages: `.input`/`.local` declarations parse as placeholders; only the pattern shows.
    if syntax == Syntax::Mf2 && message.trim_start().starts_with('.') {
        if nodes.iter().any(|n| matches!(n, Node::Choice(_))) {
            nodes.retain(|n| matches!(n, Node::Choice(_)));
        } else if let Some(i) = message.find("{{") {
            nodes = parse(&message[i..], syntax)?;
        }
    }
    // Variables and the union of their option keys, in first-seen order.
    let mut vars: Vec<(String, Kind, BTreeSet<String>)> = vec![];
    super::walk(&nodes, &mut |n| {
        if let Node::Choice(c) = n {
            let keys = c.options.iter().map(|(k, _)| k.clone());
            match vars.iter_mut().find(|v| v.0 == c.name) {
                Some(v) => v.2.extend(keys),
                None => vars.push((c.name.clone(), c.kind, keys.collect())),
            }
        }
    });
    // Cases per variable: (case label, injected value).
    let axes: Vec<(String, Vec<(String, String)>)> = vars
        .into_iter()
        .map(|(name, kind, keys)| {
            let mut cases: Vec<(String, String)> = vec![];
            let mut push = |c: String, v: String| {
                if !cases.iter().any(|(x, _)| *x == c) {
                    cases.push((c, v));
                }
            };
            match kind {
                Kind::Plural | Kind::Ordinal => {
                    let ws = if kind == Kind::Plural {
                        witnesses(locale)
                    } else {
                        ordinal_witnesses(locale)
                    };
                    for (c, w) in &ws {
                        push(c.as_str().into(), (*w).into());
                    }
                    for k in &keys {
                        if let Some(n) = exact(k) {
                            push(format!("={n}"), n.into());
                        } else if let Some(c) = ALL.iter().find(|c| c.as_str() == k) {
                            // A category the locale doesn't use (ordinals of other languages).
                            push(k.clone(), witness(locale, *c).into());
                        }
                    }
                }
                Kind::Select | Kind::Unknown => {
                    for k in keys.iter().filter(|k| *k != "other") {
                        push(k.clone(), k.clone());
                    }
                    push("other".into(), "other".into());
                }
            }
            (name, cases)
        })
        .collect();

    let mut out: Vec<Branch> = vec![];
    let mut seen = BTreeSet::new();
    let total: usize = axes.iter().map(|(_, c)| c.len()).product();
    for i in 0..total.min(MAX_BRANCHES * 4) {
        // Mixed-radix counter over the axes.
        let mut rest = i;
        let pick: BTreeMap<&str, &(String, String)> = axes
            .iter()
            .map(|(name, cases)| {
                let c = &cases[rest % cases.len()];
                rest /= cases.len();
                (name.as_str(), c)
            })
            .collect();
        let mut r = Render {
            pick: &pick,
            visited: vec![],
            plural: None,
        };
        let mut text = String::new();
        r.nodes(&nodes, None, &mut text);
        // Variables never reached in this combination don't make it a different branch.
        let cases: Vec<(String, String)> = axes
            .iter()
            .filter(|(n, _)| r.visited.contains(&n.as_str()))
            .map(|(n, _)| (n.clone(), pick[n.as_str()].0.clone()))
            .collect();
        let id = cases
            .iter()
            .map(|(n, c)| format!("{n}={c}"))
            .collect::<Vec<_>>()
            .join(",");
        if !seen.insert(id.clone()) {
            continue;
        }
        let values = cases
            .iter()
            .map(|(n, _)| (n.clone(), pick[n.as_str()].1.clone()))
            .collect();
        let category = match (r.plural, cases.is_empty()) {
            (Some(c), _) => c.to_string(),
            (None, false) => "select".into(),
            (None, true) => String::new(),
        };
        out.push(Branch {
            id,
            cases,
            values,
            category,
            text,
        });
        if out.len() >= MAX_BRANCHES {
            break;
        }
    }
    Ok(out)
}

const ALL: [PluralCategory; 6] = [
    PluralCategory::Zero,
    PluralCategory::One,
    PluralCategory::Two,
    PluralCategory::Few,
    PluralCategory::Many,
    PluralCategory::Other,
];

/// `=0` (ICU) / `0` (MF2) => `0`.
fn exact(key: &str) -> Option<&str> {
    let n = key.strip_prefix('=').unwrap_or(key);
    n.parse::<f64>().is_ok().then_some(n)
}

struct Render<'a> {
    pick: &'a BTreeMap<&'a str, &'a (String, String)>,
    visited: Vec<&'a str>,
    plural: Option<&'a str>,
}

impl<'a> Render<'a> {
    fn nodes(&mut self, nodes: &'a [Node], pound: Option<&'a str>, out: &mut String) {
        for n in nodes {
            match n {
                Node::Text(t) => out.push_str(t),
                Node::Pound => out.push_str(pound.unwrap_or("#")),
                Node::Arg { name, format } => match self.pick.get(name.as_str()) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        // Sample value as `glot measure`/`render_sample` would show it.
                        let icu = match format {
                            Some(f) if !f.starts_with(':') => format!("{{{name}, {f}}}"),
                            _ => format!("{{{name}}}"),
                        };
                        out.push_str(&render_sample(&icu, Syntax::Icu));
                    }
                },
                Node::Choice(c) => {
                    let Some((case, value)) = self.pick.get(c.name.as_str()).copied() else {
                        continue;
                    };
                    if !self.visited.contains(&c.name.as_str()) {
                        self.visited.push(&c.name);
                    }
                    let numeric = matches!(c.kind, Kind::Plural | Kind::Ordinal);
                    if numeric {
                        self.plural = Some(case);
                    }
                    // ICU: an exact `=N` wins over the category, then the category, then `other`.
                    let body = c
                        .options
                        .iter()
                        .find(|(k, _)| numeric && exact(k) == Some(value.as_str()))
                        .or_else(|| {
                            c.options
                                .iter()
                                .find(|(k, _)| !case.starts_with('=') && k == case)
                        })
                        .or_else(|| c.options.iter().find(|(k, _)| k == "other"));
                    if let Some((_, b)) = body {
                        let p = if numeric { Some(value.as_str()) } else { pound };
                        self.nodes(b, p, out);
                    }
                }
                Node::Open(_) | Node::Close(_) | Node::Void(_) | Node::Ref(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(msg: &str, locale: &str, syntax: Syntax) -> Vec<(String, String)> {
        enumerate(msg, locale, syntax)
            .unwrap()
            .into_iter()
            .map(|b| (b.id, b.text))
            .collect()
    }

    #[test]
    fn polish_arabic_japanese() {
        let pl = "{n, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}";
        assert_eq!(
            ids(pl, "pl", Syntax::Icu),
            [
                ("n=one".into(), "1 plik".into()),
                ("n=few".into(), "3 pliki".into()),
                ("n=many".into(), "11 plików".into()),
                ("n=other".into(), "1.5 pliku".into()),
            ]
        );
        // `few` missing from the message: the branch still exists and falls back to `other`.
        let b = enumerate(
            "{n, plural, one {# plik} other {# plików}}",
            "pl",
            Syntax::Icu,
        )
        .unwrap();
        assert_eq!(b.len(), 4);
        assert_eq!(b[1].text, "3 plików");

        let ar = "{n, plural, =0 {لا شيء} zero {#} one {واحد} two {اثنان} few {# قليل} many {# كثير} other {# آخر}}";
        let b = enumerate(ar, "ar", Syntax::Icu).unwrap();
        let values: Vec<&str> = b.iter().map(|b| b.values["n"].as_str()).collect();
        assert_eq!(values, ["0", "1", "2", "3", "11", "100", "0"]);
        // zero category with witness 0 hits the exact `=0` (ICU precedence), so does the `=0` case.
        assert_eq!(b[0].text, "لا شيء");
        assert_eq!(b[6].id, "n==0");
        assert_eq!(b[4].text, "11 كثير");

        assert_eq!(
            ids("{n, plural, other {#件}}", "ja", Syntax::Icu),
            [("n=other".into(), "5件".into())]
        );
        assert_eq!(
            ids("Hello <b>{name}</b>", "fr", Syntax::Icu),
            [("".into(), "Hello Alex Martin".into())]
        );
    }

    #[test]
    fn nested_select_and_ordinal() {
        let msg = "{gender, select, female {{n, plural, one {Elle a # fichier} other {Elle a # fichiers}}} other {Ils ont des fichiers}}";
        let b = ids(msg, "fr", Syntax::Icu);
        // female × (one, many, other) + other (n never reached: one branch, not three).
        assert_eq!(
            b.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(),
            [
                "gender=female,n=one",
                "gender=other",
                "gender=female,n=many",
                "gender=female,n=other"
            ]
        );
        assert_eq!(b[0].1, "Elle a 1 fichier");
        let full = enumerate(msg, "fr", Syntax::Icu).unwrap();
        assert_eq!(
            (full[0].category.as_str(), full[1].category.as_str()),
            ("one", "select")
        );
        assert_eq!(b[2].1, "Elle a 1000000 fichiers");

        let ord = "{pos, selectordinal, one {#st} two {#nd} few {#rd} other {#th}}";
        assert_eq!(
            ids(ord, "en", Syntax::Icu)
                .into_iter()
                .map(|(_, t)| t)
                .collect::<Vec<_>>(),
            ["1st", "2nd", "3rd", "4th"]
        );
        // fr ordinals: one/other only.
        assert_eq!(
            ids(
                "{p, selectordinal, one {#er} other {#e}}",
                "fr",
                Syntax::Icu
            )
            .len(),
            2
        );
    }

    #[test]
    fn mf2_match() {
        let msg = ".input {$count :number}\n.match $count\n0 {{Aucun fichier}}\none {{{$count} fichier}}\n* {{{$count} fichiers}}";
        let b = ids(msg, "fr", Syntax::Mf2);
        assert_eq!(
            b,
            [
                ("count=one".into(), "1 fichier".into()),
                ("count=many".into(), "1000000 fichiers".into()),
                ("count=other".into(), "5 fichiers".into()),
                ("count==0".into(), "Aucun fichier".into()),
            ]
        );
    }
}
