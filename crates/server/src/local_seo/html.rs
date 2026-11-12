//! Reading a page's search-facing content out of raw HTML, and writing an override back into it.
use lol_html::{element, html_content::ContentType, text};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::validate::{Content, Page};

const TEXT_MAX: usize = 2000;

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a crawler reads without JavaScript: title, description, first `<h1>`, `<h2>`s and a text excerpt.
pub fn extract(html: &str) -> Page {
    let seo = glot_core::edge::seo::inspect(html);
    let h1 = RefCell::new(Vec::<String>::new());
    let h2 = RefCell::new(Vec::<String>::new());
    let body = RefCell::new(String::new());
    let heading = |store: &RefCell<Vec<String>>,
                   t: &lol_html::html_content::TextChunk,
                   open: &RefCell<bool>| {
        let mut s = store.borrow_mut();
        if !*open.borrow() {
            s.push(String::new());
            *open.borrow_mut() = true;
        }
        s.last_mut().unwrap().push_str(t.as_str());
        if t.last_in_text_node() {
            s.last_mut().unwrap().push(' ');
        }
    };
    let (in_h1, in_h2) = (RefCell::new(false), RefCell::new(false));
    let settings = lol_html::RewriteStrSettings::new()
        .append_element_content_handler(element!("h1, h2", |el| {
            // A new heading starts a new entry, even when the previous one had nested markup.
            if el.tag_name() == "h1" {
                *in_h1.borrow_mut() = false;
            } else {
                *in_h2.borrow_mut() = false;
            }
            Ok(())
        }))
        .append_element_content_handler(text!("h1, h1 *", |t| {
            heading(&h1, t, &in_h1);
            Ok(())
        }))
        .append_element_content_handler(text!("h2, h2 *", |t| {
            heading(&h2, t, &in_h2);
            Ok(())
        }))
        .append_element_content_handler(text!(
            "body, body *:not(script):not(style):not(noscript)",
            |t| {
                let mut b = body.borrow_mut();
                if b.len() < TEXT_MAX * 2 {
                    b.push_str(t.as_str());
                    if t.last_in_text_node() {
                        b.push(' ');
                    }
                }
                Ok(())
            }
        ));
    let _ = lol_html::rewrite_str(html, settings);
    let clean = |v: Vec<String>| -> Vec<String> {
        v.iter()
            .map(|s| squash(s))
            .filter(|s| !s.is_empty())
            .collect()
    };
    let h1 = clean(h1.into_inner());
    Page {
        title: seo.title.unwrap_or_default(),
        description: seo.description.map(|d| squash(&d)).unwrap_or_default(),
        h1: h1.into_iter().next().unwrap_or_default(),
        h2: clean(h2.into_inner()).into_iter().take(12).collect(),
        text: squash(&body.into_inner()).chars().take(TEXT_MAX).collect(),
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `html` with the override written in: `<title>`, description, Open Graph / Twitter title and description, the
/// first `<h1>`, the `<h2>`s in order, and a `FAQPage` JSON-LD block. Fields the override leaves out are untouched.
/// On any parse error the page is returned as it came.
// ponytail: the FAQ is added as structured data only. Rendering visible questions needs a place in the customer's
// layout (a selector in settings); until then FAQ rich results depend on the page already showing those answers.
pub fn apply(html: &str, c: &Content) -> String {
    let h1_done = RefCell::new(false);
    let h2_at = RefCell::new(0usize);
    let has_description = Rc::new(Cell::new(false));
    let mut handlers = vec![];
    if let Some(t) = &c.title {
        handlers.push(element!("title", |el| {
            el.set_inner_content(t, ContentType::Text);
            Ok(())
        }));
        handlers.push(element!(
            r#"meta[property="og:title"], meta[name="twitter:title"]"#,
            |el| {
                el.set_attribute("content", t)?;
                Ok(())
            }
        ));
    }
    if let Some(d) = &c.description {
        handlers.push(element!(
            r#"meta[name="description"], meta[property="og:description"], meta[name="twitter:description"]"#,
            |el| {
                if el.get_attribute("name").is_some_and(|n| n.eq_ignore_ascii_case("description")) {
                    has_description.set(true);
                }
                el.set_attribute("content", d)?;
                Ok(())
            }
        ));
    }
    if let Some(h) = &c.h1 {
        handlers.push(element!("h1", |el| {
            if !h1_done.replace(true) {
                el.set_inner_content(h, ContentType::Text);
            }
            Ok(())
        }));
    }
    if !c.h2.is_empty() {
        handlers.push(element!("h2", |el| {
            let i = h2_at.replace_with(|i| *i + 1);
            if let Some(h) = c.h2.get(i) {
                el.set_inner_content(h, ContentType::Text);
            }
            Ok(())
        }));
    }
    // Appended at the end of <head>: by then the handlers above know whether a description tag exists.
    handlers.push(element!("head", |el| {
        let (d, faq) = (c.description.clone(), c.faq.clone());
        let has_description = has_description.clone();
        el.on_end_tag(lol_html::end_tag!(move |tag| {
            if let Some(d) = &d
                && !has_description.get()
            {
                tag.before(
                    &format!(
                        r#"<meta name="description" content="{}">"#,
                        esc(d).replace('"', "&quot;")
                    ),
                    ContentType::Html,
                );
            }
            if !faq.is_empty() {
                let ld = json!({
                    "@context": "https://schema.org",
                    "@type": "FAQPage",
                    "mainEntity": faq.iter().map(|f| json!({
                        "@type": "Question",
                        "name": f.question,
                        "acceptedAnswer": {"@type": "Answer", "text": f.answer},
                    })).collect::<Vec<_>>(),
                });
                // `</` cannot appear inside a script element.
                let body = ld.to_string().replace("</", "<\\/");
                tag.before(
                    &format!(
                        r#"<script type="application/ld+json" data-glot-local-seo>{body}</script>"#
                    ),
                    ContentType::Html,
                );
            }
            Ok(())
        }))?;
        Ok(())
    }));
    // ponytail: same builder pattern as crates/core/src/edge/seo.rs (struct fields are private in lol_html).
    let settings = handlers
        .into_iter()
        .fold(lol_html::RewriteStrSettings::new(), |s, h| {
            s.append_element_content_handler(h)
        });
    lol_html::rewrite_str(html, settings).unwrap_or_else(|_| html.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_seo::validate::Faq;

    const HTML: &str = r#"<!doctype html><html lang="fr"><head><title>Recadrez vos vidéos | ClipOnIt</title>
<meta name="description" content="Recadrez vos vidéos en un clic">
<meta property="og:title" content="Recadrez vos vidéos | ClipOnIt">
</head><body><h1>Recadrez <em>vos</em> vidéos</h1><h2>Rapide</h2><p>Importez une vidéo.</p><h2>Simple</h2><h1>Autre</h1></body></html>"#;

    #[test]
    fn extracts_what_a_crawler_reads() {
        let p = extract(HTML);
        assert_eq!(p.title, "Recadrez vos vidéos | ClipOnIt");
        assert_eq!(p.description, "Recadrez vos vidéos en un clic");
        assert_eq!(p.h1, "Recadrez vos vidéos");
        assert_eq!(p.h2, ["Rapide", "Simple"]);
        assert!(p.text.contains("Importez une vidéo."), "{}", p.text);
    }

    #[test]
    fn applies_only_the_fields_of_the_override() {
        let c = Content {
            title: Some("Cortar video en línea | ClipOnIt".into()),
            h1: Some("Cortar <video> en línea".into()),
            faq: vec![Faq {
                question: "¿Cómo?".into(),
                answer: "Así </script>".into(),
            }],
            ..Default::default()
        };
        let out = apply(HTML, &c);
        assert!(
            out.contains("<title>Cortar video en línea | ClipOnIt</title>"),
            "{out}"
        );
        assert!(
            out.contains(
                r#"<meta property="og:title" content="Cortar video en línea | ClipOnIt">"#
            ),
            "{out}"
        );
        // Text is escaped, only the first h1 changes, the description and the h2s are untouched.
        assert!(
            out.contains("<h1>Cortar &lt;video&gt; en línea</h1>"),
            "{out}"
        );
        assert!(out.contains("<h1>Autre</h1>"));
        assert!(out.contains(r#"content="Recadrez vos vidéos en un clic""#));
        assert!(out.contains("<h2>Rapide</h2>"));
        assert!(out.contains(r#""@type":"FAQPage""#));
        assert!(!out.contains("Así </script>"));
        assert_eq!(extract(&out).title, "Cortar video en línea | ClipOnIt");
        // Nothing to apply: byte-identical.
        assert_eq!(apply(HTML, &Content::default()), HTML);
        // A page without a description tag gets one.
        let d = Content {
            description: Some("Corta \"ya\"".into()),
            ..Default::default()
        };
        let out = apply(
            "<html><head><title>x</title></head><body></body></html>",
            &d,
        );
        assert!(
            out.contains(r#"<meta name="description" content="Corta &quot;ya&quot;"></head>"#),
            "{out}"
        );
    }
}
