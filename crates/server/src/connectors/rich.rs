//! Structured values → translation units and back, without touching the structure.
//!
//! - HTML (`Format::Html`, also Gutenberg block HTML): segments of `glot_core::edge::segment` (inline tags as
//!   `<0>…</0>`, values masked as `{0}`); comments (`<!-- wp:paragraph {"x":1} -->`) and markup are kept byte
//!   for byte by `translate_html`.
//! - Rich text JSON (`Format::RichText`): every text node is a unit, translated in place; marks, links,
//!   embeds and keys stay. Text nodes: `{"nodeType":"text","value":…}` (Contentful), `{"_type":"span","text":…}`
//!   (Sanity Portable Text), `{"type":"text","text":…}` (Strapi Blocks, Lexical, ProseMirror/Tiptap),
//!   `{"type":"extended-text","text":…}` (Ghost's Lexical).
//!
//! Unit key = `{item key}#{12 hex of sha256(text)}`: the same text in the same field is translated once.
use glot_core::edge::segment::{SegmentOpts, segment, translate_html};
use serde_json::Value;

use super::Format;

fn unit_key(key: &str, text: &str) -> String {
    format!("{key}#{}", &crate::auth::sha256_hex(text)[..12])
}

/// Text node of a rich text document: the field holding its text.
fn text_field(o: &serde_json::Map<String, Value>) -> Option<&'static str> {
    let is = |k: &str, v: &str| o.get(k).and_then(Value::as_str) == Some(v);
    if is("nodeType", "text") && o.get("value").is_some_and(Value::is_string) {
        Some("value")
    } else if (is("_type", "span") || is("type", "text") || is("type", "extended-text"))
        && o.get("text").is_some_and(Value::is_string)
    {
        Some("text")
    } else if is("type", "text") && o.get("value").is_some_and(Value::is_string) {
        // Shopify rich text metafields: {"type":"root","children":[{"type":"paragraph","children":[{"type":"text","value":…}]}]}
        Some("value")
    } else {
        None
    }
}

fn walk_texts<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::Object(o) => {
            if let Some(f) = text_field(o) {
                out.push(o[f].as_str().unwrap_or_default());
            }
            o.values().for_each(|c| walk_texts(c, out));
        }
        Value::Array(a) => a.iter().for_each(|c| walk_texts(c, out)),
        _ => {}
    }
}

fn walk_set(v: &mut Value, tr: &mut dyn FnMut(&str) -> Option<String>, missing: &mut bool) {
    match v {
        Value::Object(o) => {
            if let Some(f) = text_field(o) {
                let src = o[f].as_str().unwrap_or_default().to_string();
                if has_text(&src) {
                    match tr(&src) {
                        Some(t) => {
                            // Keep the node's own leading/trailing spaces (they separate marks).
                            let lead = &src[..src.len() - src.trim_start().len()];
                            let trail = &src[src.trim_end().len()..];
                            o[f] = Value::String(format!("{lead}{}{trail}", t.trim()));
                        }
                        None => *missing = true,
                    }
                }
            }
            for (_, c) in o.iter_mut() {
                walk_set(c, tr, missing);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| walk_set(c, tr, missing)),
        _ => {}
    }
}

fn has_text(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

/// (unit key, source text) of a value, deduplicated, in document order.
pub fn units(
    key: &str,
    format: &Format,
    source: &str,
    source_locale: &str,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = match format {
        Format::Plain => {
            if source.trim().is_empty() {
                vec![]
            } else {
                vec![(key.to_string(), source.to_string())]
            }
        }
        Format::Html => segment(source, &SegmentOpts::new(source_locale))
            .into_iter()
            .map(|s| (unit_key(key, &s.text), s.text))
            .collect(),
        Format::RichText(doc) => {
            let mut t = vec![];
            walk_texts(doc, &mut t);
            t.into_iter()
                .filter(|s| has_text(s))
                .map(|s| (unit_key(key, s.trim()), s.trim().to_string()))
                .collect()
        }
    };
    let mut seen = std::collections::HashSet::new();
    out.retain(|(k, _)| seen.insert(k.clone()));
    out
}

/// The translated value, `tr(unit key)` giving each unit's translation; `None` while any unit is missing
/// (never a half-translated page). For rich text, the translated document serialized as JSON.
pub fn rebuild(
    key: &str,
    format: &Format,
    source: &str,
    source_locale: &str,
    tr: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    match format {
        Format::Plain => tr(key),
        Format::Html => {
            let opts = SegmentOpts::new(source_locale);
            let mut missing = false;
            let out = translate_html(source, &opts, |s| {
                let t = tr(&unit_key(key, &s.text));
                missing |= t.is_none();
                t
            });
            (!missing).then_some(out)
        }
        Format::RichText(doc) => {
            let mut doc = doc.clone();
            let mut missing = false;
            walk_set(
                &mut doc,
                &mut |s| tr(&unit_key(key, s.trim())),
                &mut missing,
            );
            (!missing).then(|| doc.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn fr(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |k| map.get(k).cloned()
    }

    #[test]
    fn gutenberg_blocks_keep_comments() {
        let html = "<!-- wp:heading {\"level\":2} -->\n<h2 class=\"wp-block-heading\">Hello <strong>world</strong></h2>\n<!-- /wp:heading -->\n\n<!-- wp:paragraph -->\n<p>Read our <a href=\"/terms\">terms</a>.</p>\n<!-- /wp:paragraph -->";
        let u = units("wordpress:post:1:content", &Format::Html, html, "en");
        let texts: Vec<_> = u.iter().map(|x| x.1.as_str()).collect();
        assert_eq!(texts, ["Hello <0>world</0>", "Read our <0>terms</0>."]);
        let mut m = HashMap::new();
        m.insert(u[0].0.clone(), "Bonjour <0>le monde</0>".to_string());
        assert_eq!(
            rebuild(
                "wordpress:post:1:content",
                &Format::Html,
                html,
                "en",
                &fr(&m)
            ),
            None,
            "partial = not pushed"
        );
        m.insert(u[1].0.clone(), "Lisez nos <0>conditions</0>.".to_string());
        let out = rebuild(
            "wordpress:post:1:content",
            &Format::Html,
            html,
            "en",
            &fr(&m),
        )
        .unwrap();
        assert_eq!(
            out,
            "<!-- wp:heading {\"level\":2} -->\n<h2 class=\"wp-block-heading\">Bonjour <strong>le monde</strong></h2>\n<!-- /wp:heading -->\n\n<!-- wp:paragraph -->\n<p>Lisez nos <a href=\"/terms\">conditions</a>.</p>\n<!-- /wp:paragraph -->"
        );
    }

    #[test]
    fn rich_text_nodes_in_place() {
        let doc = json!({"nodeType": "document", "content": [{"nodeType": "paragraph", "content": [
            {"nodeType": "text", "value": "Free ", "marks": [{"type": "bold"}]},
            {"nodeType": "hyperlink", "data": {"uri": "https://x.io"}, "content": [
                {"nodeType": "text", "value": "shipping", "marks": []}]},
            {"nodeType": "text", "value": " ", "marks": []}]}]});
        let f = Format::RichText(doc);
        let u = units("contentful:entry:1:body", &f, "", "en");
        assert_eq!(
            u.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
            ["Free", "shipping"]
        );
        let m: HashMap<_, _> = [
            (u[0].0.clone(), "Livraison".to_string()),
            (u[1].0.clone(), "gratuite".to_string()),
        ]
        .into();
        let out: Value = serde_json::from_str(
            &rebuild("contentful:entry:1:body", &f, "", "en", &fr(&m)).unwrap(),
        )
        .unwrap();
        let p = &out["content"][0]["content"];
        assert_eq!(p[0]["value"], "Livraison ");
        assert_eq!(p[0]["marks"][0]["type"], "bold");
        assert_eq!(p[1]["data"]["uri"], "https://x.io");
        assert_eq!(p[1]["content"][0]["value"], "gratuite");
        // Portable Text / Strapi blocks.
        let pt = Format::RichText(
            json!([{"_type": "block", "children": [{"_type": "span", "text": "Hi", "marks": ["strong"]}]},
            {"type": "paragraph", "children": [{"type": "text", "text": "Bye"}]}]),
        );
        assert_eq!(units("k", &pt, "", "en").len(), 2);
    }
}
