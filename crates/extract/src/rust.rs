//! Rust sources (Leptos `view!`, Yew `html!`, Dioxus `rsx!`), by literal patterns (no `syn`).
//!
//! Usages: `t("key")`, `tn("key", n)` and the other `t*` helpers of hand-rolled tables, rust-i18n /
//! dioxus-i18n `t!("key")`, i18n-embed `fl!(loader, "key")`, leptos_i18n `t!(i18n, home.title)`
//! (path => `home.title`), plus `[extract] functions`.
//! Hardcoded copy: string literals that are text nodes (`<p>"Hello"</p>`, `{ "Hello" }`, rsx
//! `p { "Hello" }`) or user-visible attributes (`placeholder="…"`, rsx `title: "…"`) inside those
//! macros; literals passed to functions, `class=` and the like are code.
// ponytail: no Rust parser: a key built at runtime (`t(&format!(…))`) is skipped, never mis-keyed;
// `format!` arguments inside a macro are not copy (they follow `(` or `,`).
use crate::{ExtractReport, Hardcoded, Usage, scan};
use glot_core::context::SourceRef;

const FNS: &[&str] = &[
    "t",
    "tn",
    "tc",
    "tl",
    "tnm",
    "tcl",
    "tr",
    "t!",
    "tr!",
    "fl!",
    "t_string!",
    "td!",
    "td_string!",
    "t_display!",
];
const MACROS: &[&str] = &["view!", "html!", "rsx!"];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A string literal (`"…"` with escapes, `r#"…"#`) starting at `i`: its value and end.
fn literal(s: &str, i: usize) -> Option<(String, usize)> {
    let r = &s[i..];
    if let Some(raw) = r.strip_prefix('r') {
        let h = raw.len() - raw.trim_start_matches('#').len();
        let body = raw[h..].strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(h));
        let n = body.find(&close)?;
        return Some((body[..n].to_string(), i + 2 + h + n + close.len()));
    }
    let body = r.strip_prefix('"')?;
    let (mut out, mut esc) = (String::new(), false);
    for (k, c) in body.char_indices() {
        match (esc, c) {
            (true, 'n') => out.push('\n'),
            (true, c) => out.push(c),
            (false, '\\') => {
                esc = true;
                continue;
            }
            (false, '"') => return Some((out, i + 1 + k + 1)),
            (false, c) => out.push(c),
        }
        esc = false;
    }
    None
}

/// Top-level arguments of the call whose `(` is at `open` (raw text, trimmed).
fn args(s: &str, open: usize) -> Vec<&str> {
    let (mut out, mut depth, mut start, mut i) = (vec![], 0i32, open + 1, open);
    let b = s.as_bytes();
    while i < s.len() {
        match b[i] {
            b'"' => {
                i = literal(s, i).map_or(i + 1, |(_, e)| e);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    out.push(s[start..i].trim());
                    return out;
                }
            }
            b',' if depth == 1 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    vec![]
}

/// Page-local lookups (`fn lt(key: &str) -> String`, `fn ht(k: &'static str)`).
// ponytail: same-file definitions only; a helper imported from elsewhere goes in `[extract] functions`.
fn lookup_fns(src: &str) -> Vec<String> {
    src.match_indices("fn ")
        .filter_map(|(i, _)| {
            let rest = &src[i + 3..];
            let n = rest.find(|c: char| !is_ident(c))?;
            let params = rest[n..].trim_start().strip_prefix('(')?.trim_start();
            let (param, ty) = params.split_once(':')?;
            // A lookup: its body searches a table or calls `t(key)` (not `env(key)`).
            let mut end = rest.find("\n}").unwrap_or(rest.len()).min(600);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let body = &rest[..end];
            let looks_up = [".find(|", "t(key", "tl(key", "t(k)", "tl(k,"]
                .iter()
                .any(|p| body.contains(p));
            (matches!(param.trim(), "key" | "k" | "id")
                && ty.trim_start().starts_with('&')
                && looks_up
                && !FNS.contains(&&rest[..n]))
            .then(|| rest[..n].to_string())
        })
        .collect()
}

fn line(s: &str, i: usize) -> u32 {
    s[..i].matches('\n').count() as u32 + 1
}

pub(crate) fn scan(path: &str, src: &str, functions: &[String]) -> ExtractReport {
    let mut r = ExtractReport {
        files_scanned: 1,
        ..Default::default()
    };
    let local = lookup_fns(src);
    let names: Vec<&str> = FNS
        .iter()
        .copied()
        .chain(functions.iter().map(String::as_str))
        .chain(local.iter().map(String::as_str))
        .collect();
    let mut pairs = 0;
    for name in names {
        for (i, _) in src.match_indices(name) {
            // Not `obj.t(`, `my_t(`, nor the helper's own `fn t(`.
            if src[..i]
                .chars()
                .next_back()
                .is_some_and(|c| is_ident(c) || c == '.')
                || src[i + name.len()..].starts_with(is_ident)
                || src[..i].trim_end().ends_with("fn")
            {
                continue;
            }
            let rest = &src[i + name.len()..];
            let Some(open) = rest
                .find(|c: char| !c.is_whitespace())
                .filter(|&k| rest[k..].starts_with('('))
            else {
                continue;
            };
            let a = args(src, i + name.len() + open);
            let lit = |x: &str| literal(x, 0).filter(|(_, e)| *e == x.len()).map(|(v, _)| v);
            // `tr("Bonjour", "Hello")`: translations inline in code, not a key.
            if let [x, y, ..] = a.as_slice()
                && lit(x).is_some()
                && lit(y).is_some()
                && !name.ends_with('!')
            {
                pairs += 1;
                continue;
            }
            // `t(&format!("role_{}", r))`: keys under `role_` count as used.
            if let Some(f) = a
                .first()
                .and_then(|x| x.trim_start_matches('&').strip_prefix("format!("))
                && let Some((v, _)) = literal(f.trim_start(), 0)
                && let Some(p) = v.split('{').next().filter(|p| !p.is_empty())
            {
                r.dynamic_prefixes.insert(p.to_string());
            }
            let key = match a.as_slice() {
                [first, ..] if lit(first).is_some() => lit(first),
                // `fl!(loader, "key")`, leptos_i18n `t!(i18n, home.title)`.
                [_, second, ..] if name.ends_with('!') => lit(second).or_else(|| {
                    second
                        .split('.')
                        .all(|p| !p.is_empty() && p.chars().all(is_ident))
                        .then(|| second.to_string())
                }),
                _ => None,
            };
            let at = SourceRef {
                path: path.to_string(),
                line: line(src, i),
            };
            match key {
                Some(k) => r.usages.entry(k).or_default().push(Usage {
                    at,
                    ui_role: None,
                    component: None,
                }),
                None if !a.is_empty() => r.dynamic.push(at),
                None => {}
            }
        }
    }
    if pairs > 0 {
        r.warnings.push(format!(
            "{path}: {pairs} inline translation pairs (`tr(\"…\", \"…\")`): glot cannot add a locale to them; move them into the string table and call t(key)"
        ));
    }
    r.hardcoded
        .extend(spots(path, src).into_iter().map(|s| s.copy));
    r
}

/// How a spot is replaced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Wrap {
    /// A text node: `{move || t("key")}`.
    Text,
    /// An attribute value: `move || t("key")`.
    Attr,
    /// A string expression: `t("key")` (`closure`: the whole value, kept reactive with `move ||`).
    Call { closure: bool },
}

/// A hardcoded literal (or an inline-translation expression) and its byte span.
#[derive(Debug, Clone)]
pub struct Spot {
    pub start: usize,
    pub end: usize,
    /// `placeholder="…"` rather than a text node.
    pub attr: bool,
    /// Replaceable without changing a type (see [`crate::rust_view`]); otherwise `copy.reason`
    /// says why and the developer gets the code to write.
    pub clean: bool,
    pub wrap: Wrap,
    /// Whitespace around the text: kept in the code, never in the catalog.
    pub pre: String,
    pub post: String,
    /// Translations found in the code (`if en { … } else { … }`, `tr("fr", "en")`): locale => text.
    pub translations: std::collections::BTreeMap<String, String>,
    /// Context for the translator (a sentence built around a value).
    pub note: Option<String>,
    pub copy: Hardcoded,
}

impl Spot {
    /// The code that replaces the spot: `f` is `t` or a path to it.
    pub fn replacement(&self, f: &str, key: &str) -> String {
        let call = format!("{f}({key:?})");
        let edges = !self.pre.is_empty() || !self.post.is_empty();
        let spaced = || {
            format!(
                "format!(\"{}{{}}{}\", {call})",
                self.pre.escape_default(),
                self.post.escape_default()
            )
        };
        let lit = |s: &str| {
            if s.is_empty() {
                String::new()
            } else {
                format!("{s:?}")
            }
        };
        match self.wrap {
            Wrap::Text => format!("{}{{move || {call}}}{}", lit(&self.pre), lit(&self.post)),
            Wrap::Attr | Wrap::Call { closure: true } if edges => format!("move || {}", spaced()),
            Wrap::Attr | Wrap::Call { closure: true } => format!("move || {call}"),
            Wrap::Call { closure: false } if edges => spaced(),
            Wrap::Call { closure: false } => call,
        }
    }
}

/// Spots of a file plus the edits that go with the clean ones (`"".to_string()` on the non-copy
/// literals sharing a branch tree with a rewritten one).
#[derive(Debug, Default)]
pub struct Analysis {
    pub spots: Vec<Spot>,
    pub aux: Vec<(usize, usize, String)>,
}

/// Every hardcoded literal of the `view!` / `html!` / `rsx!` bodies of `src`.
pub fn spots(path: &str, src: &str) -> Vec<Spot> {
    analyze(path, src).spots
}

/// `view!` through token trees (exact placement of every literal); `html!` / `rsx!`, and files
/// that do not tokenize, through the literal scanner (reported, never rewritten).
pub fn analyze(path: &str, src: &str) -> Analysis {
    let view = crate::rust_view::analyze(path, src);
    let mut out = vec![];
    for m in MACROS {
        if *m == "view!" && view.is_some() {
            continue;
        }
        for (i, _) in src.match_indices(m) {
            if src[..i].chars().next_back().is_some_and(is_ident) {
                continue;
            }
            let Some(open) = src[i + m.len()..].find('{').map(|k| i + m.len() + k) else {
                continue;
            };
            if !src[i + m.len()..open].trim().is_empty() {
                continue;
            }
            hardcoded(path, src, open, m, &mut out);
        }
    }
    let mut aux = vec![];
    if let Some(v) = view {
        out.extend(v.spots);
        aux = v.aux;
    }
    // A macro nested in another one is scanned twice.
    out.sort_by_key(|s| s.start);
    out.dedup_by_key(|s| s.start);
    Analysis { spots: out, aux }
}

/// Key for `text` shown in the file `path` (`""`: shared by several files => `common.…`).
pub fn suggest_key(path: &str, text: &str, role: &str) -> String {
    scan::suggest_key(path, text, role)
}

/// Remember the `pub fn tr(fr, en)`-style helpers of a file before analyzing the others.
pub fn register_pairs(path: &str, src: &str) {
    crate::rust_view::register(path, src)
}

/// Parse check of a rewritten file (syn), against its original.
pub fn verify(old: &str, new: &str) -> Result<(), String> {
    crate::rust_view::verify(old, new)
}

/// Copy in the macro body opened at `open`.
fn hardcoded(path: &str, src: &str, open: usize, mac: &str, out: &mut Vec<Spot>) {
    let rsx = mac == "rsx!";
    // `paren`: inside a call (`json!({ "via": … })`, `log("…")`, `format!("…")`): code, not copy.
    let (b, mut depth, mut paren, mut i) = (src.as_bytes(), 0i32, 0i32, open);
    while i < src.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            b'(' => paren += 1,
            b')' => paren -= 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = src[i..].find('\n').map_or(src.len(), |n| i + n);
                continue;
            }
            b'"' | b'r'
                if b[i] == b'"'
                    || (matches!(b.get(i + 1), Some(b'"' | b'#'))
                        && !src[..i].ends_with(is_ident)) =>
            {
                let Some((text, end)) = literal(src, i) else {
                    i += 1;
                    continue;
                };
                let before = src[..i].trim_end();
                let role = match before.chars().next_back() {
                    // `placeholder="…"` (view!/html!), `placeholder: "…"` (rsx!).
                    Some(c @ ('=' | ':')) if c == '=' || rsx => {
                        let head = before[..before.len() - 1].trim_end();
                        let n = head.len()
                            - head
                                .trim_end_matches(|c: char| is_ident(c) || c == '-')
                                .len();
                        scan::attr_role(&head[head.len() - n..].replace('_', "-"))
                    }
                    Some('>' | '}' | '{' | '"' | '#') => Some("text"),
                    // rsx `p { class: "x", "Text" }`: children follow the attributes.
                    Some(',') if rsx => Some("text"),
                    _ => None,
                };
                // `type=if submit { "submit" } else { "button" }`: one lowercase word is a value.
                let word = text.bytes().all(|c| c.is_ascii_lowercase());
                if let Some(role) = role
                    .filter(|r| paren == 0 && !(word && *r == "text"))
                    .filter(|_| scan::looks_like_text(&text))
                {
                    let attr = role != "text";
                    out.push(Spot {
                        start: i,
                        end,
                        attr,
                        // The literal scanner only reports: no rewrite without token trees.
                        clean: false,
                        wrap: if attr { Wrap::Attr } else { Wrap::Text },
                        pre: String::new(),
                        post: String::new(),
                        translations: Default::default(),
                        note: None,
                        copy: Hardcoded {
                            at: SourceRef {
                                path: path.to_string(),
                                line: line(src, i),
                            },
                            suggested_key: scan::suggest_key(path, &text, role),
                            text,
                            ui_role: Some(role.to_string()),
                            component: None,
                            reason: Some(format!(
                                "inside `{mac}`: glot rewrites Leptos `view!` only"
                            )),
                        },
                    });
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::scan;

    #[test]
    fn leptos_usages_and_copy() {
        let src = r##"
use crate::i18n::{t, tn};
pub fn t(key: &str) -> String { key.into() }
fn lt(key: &str) -> String { t(key) }
fn f() { lt("faq_q1"); env("NOT_A_KEY"); }
fn env(key: &str) -> String { std::env::var(key).unwrap() }
#[component]
pub fn Login() -> impl IntoView {
    let title = t("login_title");
    let n = tn("sup_unread", 3);
    let dynamic = t(&format!("role_{}", r));
    let pair = tr("Bonjour", "Hello");
    let x = i18n.t("not_ours");
    view! {
        // a comment "Not copy"
        <h1 class="text-xl font-bold">{title}</h1>
        <p>"Bienvenue sur ClipOnIt"</p>
        <input placeholder="Ton adresse e-mail" type="email" on:click=move |_| log("clicked here") />
        <span>{move || if ok { "Envoyé !" } else { "Échec de l'envoi" }}</span>
        <p>{t!(i18n, home.title)}</p>
        <p>{fl!(LOADER, "hello-world")}</p>
        <p>{format!("{} vidéos", n)}</p>
        <button type=if submit { "submit" } else { "button" } on:click=move |_| track(json!({ "where": "Toast menu" }))>"OK"</button>
    }
}
"##;
        let r = scan("web/src/pages/login.rs", src, &[]);
        let keys: Vec<&str> = r.usages.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "faq_q1",
                "hello-world",
                "home.title",
                "login_title",
                "sup_unread"
            ]
        );
        assert_eq!(
            r.dynamic.len(),
            2,
            "`t(&format!(…))` and `lt`'s own `t(key)`"
        );
        assert!(r.dynamic_prefixes.contains("role_"));
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        let texts: Vec<(&str, &str)> = r
            .hardcoded
            .iter()
            .map(|h| (h.text.as_str(), h.ui_role.as_deref().unwrap()))
            .collect();
        assert_eq!(
            texts,
            [
                ("Bienvenue sur ClipOnIt", "text"),
                ("Ton adresse e-mail", "placeholder"),
                ("Envoyé !", "text"),
                ("Échec de l'envoi", "text"),
                ("OK", "text"),
            ]
        );
        assert_eq!(r.hardcoded[0].at.line, 17);
    }

    /// Apply every clean spot (key = `k<n>`) and the edits that go with them.
    fn rewritten(src: &str) -> String {
        let a = super::analyze("web/src/pages/a.rs", src);
        let mut edits: Vec<(usize, usize, String)> = a
            .spots
            .iter()
            .filter(|s| s.clean)
            .enumerate()
            .map(|(n, s)| (s.start, s.end, s.replacement("t", &format!("k{n}"))))
            .chain(a.aux.clone())
            .collect();
        edits.sort();
        let (mut out, mut pos) = (String::new(), 0);
        for (start, end, with) in edits {
            out += &src[pos..start];
            out += &with;
            pos = end;
        }
        out + &src[pos..]
    }

    #[test]
    fn rewritable_spots() {
        let src = r#"fn f() { view! {
    <p>"Bienvenue"</p>
    <input placeholder="Ton e-mail" />
    <Field label="Nom" />
    <Input label="email" />
    <span>{if ok { "Envoyé !" } else { "Échec de l'envoi" }}</span>
    {move || view! { <b>"Gras"</b> }}
    <button on:keydown=move |ev| match ev.key().as_str() { "Enter" => go(), _ => {} }>{Icon::Ok.view()}"Valider"</button>
} }"#;
        let got: Vec<(&str, bool, bool)> = super::spots("web/src/pages/a.rs", src)
            .iter()
            .map(|s| (&src[s.start..s.end], s.attr, s.clean))
            .collect();
        assert_eq!(
            got,
            [
                ("\"Bienvenue\"", false, true),
                ("\"Ton e-mail\"", true, true),
                // A component prop: reported, its type belongs to the component.
                ("\"Nom\"", true, false),
                // Both branches become `t(…)`: one type, `String`.
                ("\"Envoyé !\"", false, true),
                ("\"Échec de l'envoi\"", false, true),
                ("\"Gras\"", false, true),
                // Next to a `{block}`; the `"Enter"` match pattern and `label="email"` are code.
                ("\"Valider\"", false, true),
            ]
        );
        super::verify(src, &rewritten(src)).unwrap();
    }

    #[test]
    fn branches_keep_one_type() {
        // A non-copy literal follows its siblings to `String`; an unknown branch blocks the tree.
        let src = r#"fn f() { view! {
    <i>{move || if a { "Texte visible" } else { "" }}</i>
    <b>{if a { "Autre texte".to_string() } else { t("k").to_string() }}</b>
    <u>{if a { "Encore un" } else { other }}</u>
    <a title=move || if a { "Ouvrir le menu" } else { "Fermer le menu" } class=if a { "on" } else { "off" }>"x"</a>
} }"#;
        let out = rewritten(src);
        assert!(
            out.contains(r#"{move || if a { t("k0") } else { "".to_string() }}"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{if a { t("k1") } else { t("k").to_string() }}"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{if a { "Encore un" } else { other }}"#),
            "{out}"
        );
        assert!(
            out.contains(r#"title=move || if a { t("k2") } else { t("k3") }"#),
            "{out}"
        );
        assert!(
            out.contains(r#"class=if a { "on" } else { "off" }"#),
            "{out}"
        );
        super::verify(src, &out).unwrap();
        let left: Vec<String> = super::spots("a.rs", src)
            .into_iter()
            .filter(|s| !s.clean)
            .map(|s| s.copy.text)
            .collect();
        assert_eq!(left, ["Encore un"]);
    }

    #[test]
    fn inline_translations_become_one_key() {
        crate::i18nize::set_locales("fr", &["en".to_string()]);
        let src = r#"use crate::i18n::Locale;
fn tr(fr: &'static str, en: &'static str) -> &'static str { if is_en() { en } else { fr } }
fn label() -> &'static str { tr("Hors vue", "Outside a view") }
fn f() {
    let en = is_en();
    let fr_article = article.lang == "fr";
    view! {
        <h1>{tr("Tes vidéos", "Your videos")}</h1>
        <p aria-label=move || tr("Fermer", "Close")>{move || tr("Importe une vidéo", "Import a video")}</p>
        <small>{if en { " min read" } else { " min de lecture" }}</small>
        <em>{if use_auth().locale.get() == Locale::En { "Open" } else { "Ouvrir" }}</em>
        <i>{if fr_article { "Article en français" } else { "Article in English" }}</i>
        <b>{format!("{} {}", tr("Choisir le clip", "Choose clip"), n)}</b>
        <video src=move || format!("/media/demo-{}.mp4", tr("fr", "en"))></video>
    }
}"#;
        let spots = super::spots("web/src/pages/a.rs", src);
        let got: Vec<(&str, Option<&str>, bool)> = spots
            .iter()
            .map(|s| {
                (
                    s.copy.text.as_str(),
                    s.translations.get("en").map(String::as_str),
                    s.clean,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                // Outside a view, the helper returns `&'static str`: reported with the code to write.
                ("Hors vue", Some("Outside a view"), false),
                ("Tes vidéos", Some("Your videos"), true),
                ("Fermer", Some("Close"), true),
                ("Importe une vidéo", Some("Import a video"), true),
                ("min de lecture", Some("min read"), true),
                ("Ouvrir", Some("Open"), true),
                // `fr_article` is not the interface language: two messages, no merge.
                ("Article en français", None, true),
                ("Article in English", None, true),
                // An argument of `format!` only needs `Display`: `String` fits.
                ("Choisir le clip", Some("Choose clip"), true),
            ]
        );
        let out = rewritten(src);
        assert!(out.contains(r#"<h1>{move || t("k0")}</h1>"#), "{out}");
        assert!(
            out.contains(r#"aria-label=move || t("k1")>{move || t("k2")}"#),
            "{out}"
        );
        // Surrounding whitespace stays in the code, out of the catalog.
        assert!(
            out.contains(r#"<small>{move || format!(" {}", t("k3"))}</small>"#),
            "{out}"
        );
        super::verify(src, &out).unwrap();
        crate::i18nize::set_locales("", &[]);
    }

    #[test]
    fn fragments_and_units() {
        let src = r#"fn f() { view! { <p>{pct}" du début"</p> <small>{secs}" s. "</small> <i>{lag}" ms "</i> <a>"ClipOnIt vs "{name}</a> } }"#;
        let spots = super::spots("a.rs", src);
        let got: Vec<(&str, bool, bool)> = spots
            .iter()
            .map(|s| (s.copy.text.as_str(), s.clean, s.note.is_some()))
            .collect();
        // A unit glued to a value is never a message (`" s. "` is not even copy, `" ms "` is
        // reported); a fragment is one, with a note for the translator.
        assert_eq!(
            got,
            [
                ("du début", true, true),
                ("ms", false, true),
                ("ClipOnIt vs", true, true)
            ]
        );
        assert_eq!(spots[0].replacement("t", "k"), r#"" "{move || t("k")}"#);
    }

    #[test]
    fn verify_rejects_broken_rewrites() {
        let ok =
            r#"fn f() { view! { <p on:click=move |_| go()>{if a { "x" } else { "y" }}</p> } }"#;
        super::verify(ok, ok).unwrap();
        assert!(super::verify(ok, "fn f() { view! { <p>{if a { }</p> } }").is_err());
        assert!(
            super::verify(ok, r#"fn f() { view! { <p>{if a "x" else { "y" }}</p> } }"#).is_err(),
            "a block that no longer parses as Rust"
        );
    }

    #[test]
    fn dioxus_rsx() {
        let src = "fn App() -> Element {\n    rsx! {\n        div { class: \"card\",\n            p { class: \"muted\", \"Built with Dioxus\" }\n            h1 { \"Hello, Dioxus\" }\n            input { placeholder: \"Search the docs\" }\n            p { {t!(\"intro\")} }\n        }\n    }\n}\n";
        let r = scan("src/main.rs", src, &[]);
        assert!(r.usages.contains_key("intro"));
        let texts: Vec<&str> = r.hardcoded.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(
            texts,
            ["Built with Dioxus", "Hello, Dioxus", "Search the docs"]
        );
    }
}
