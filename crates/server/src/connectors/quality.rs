//! Last check before a translation is written to a platform: what must never be translated or broken is
//! still there, byte for byte. A value that fails is not pushed; the item is reported with a fix.
//!
//! - Template code: Liquid / HubL / Twig / Handlebars `{{ … }}` and `{% … %}` (Shopify themes and emails,
//!   HubSpot modules), `${name}`, `%{name}`, `{name}`, printf `%s` / `%1$s`.
//! - WordPress shortcodes `[gallery ids="1"]…[/gallery]`: same names, same count (attributes may be translated).
//! - HTML: same tags (same count per name), same `href` / `src` values, and no segment placeholder (`<0>`)
//!   left behind by the translator.
use std::collections::BTreeMap;

type Bag = BTreeMap<String, i32>;

fn add(bag: &mut Bag, token: String) {
    *bag.entry(token).or_default() += 1;
}

/// Whitespace inside `{{ … }}` is free.
fn squeeze(s: &str) -> String {
    s.split_whitespace().collect()
}

/// Template code and variables of `s`.
fn code(s: &str) -> Bag {
    let b = s.as_bytes();
    let mut out = Bag::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &s[i..];
        let two = b.get(i + 1).copied();
        // `{{ … }}`, `{% … %}`
        if b[i] == b'{' && matches!(two, Some(b'{' | b'%')) {
            let close = if two == Some(b'{') { "}}" } else { "%}" };
            if let Some(end) = rest.find(close) {
                add(&mut out, squeeze(&rest[..end + 2]));
                i += end + 2;
                continue;
            }
        }
        // `{name}`, `${name}`, `%{name}`
        if b[i] == b'{' {
            let inner: String = rest[1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if !inner.is_empty() && rest[1 + inner.len()..].starts_with('}') {
                add(&mut out, format!("{{{inner}}}"));
                i += inner.len() + 2;
                continue;
            }
        }
        // printf `%s`, `%d`, `%1$s`
        if b[i] == b'%' {
            let digits = rest[1..].bytes().take_while(u8::is_ascii_digit).count();
            let at = 1 + digits;
            let positional = digits > 0 && rest[at..].starts_with('$');
            let conv = at + usize::from(positional);
            if (digits == 0 || positional)
                && matches!(rest.as_bytes().get(conv), Some(b's' | b'd'))
                && !rest
                    .as_bytes()
                    .get(conv + 1)
                    .is_some_and(u8::is_ascii_alphanumeric)
                && (i == 0 || !b[i - 1].is_ascii_digit() || positional)
            {
                add(&mut out, rest[..=conv].to_string());
                i += conv + 1;
                continue;
            }
        }
        // `[shortcode …]`, `[/shortcode]`: lowercase name (WordPress convention), attributes ignored.
        if b[i] == b'[' {
            let slash = usize::from(two == Some(b'/'));
            let name: String = rest[1 + slash..]
                .chars()
                .take_while(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'
                })
                .collect();
            let after = rest[1 + slash + name.len()..].chars().next();
            if name.starts_with(|c: char| c.is_ascii_lowercase())
                && matches!(after, Some(']' | ' ' | '/'))
                && let Some(end) = rest.find(']')
                && !rest[..end].contains('\n')
            {
                add(
                    &mut out,
                    format!("[{}{name}]", if slash == 1 { "/" } else { "" }),
                );
                i += end + 1;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// HTML tags (`p`, `/p`, segment placeholders `0`) and link / media targets of `s`.
fn markup(s: &str) -> (Bag, Bag) {
    let (mut tags, mut urls) = (Bag::new(), Bag::new());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[..gt];
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '-')
            .collect();
        if name.trim_start_matches('/').is_empty() || tag.starts_with('!') {
            continue;
        }
        add(&mut tags, name.to_ascii_lowercase());
        for attr in ["href=", "src="] {
            if let Some(at) = tag.find(attr) {
                let v = &tag[at + attr.len()..];
                let v = match v.chars().next() {
                    Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
                    _ => v.split_whitespace().next().unwrap_or(""),
                };
                add(&mut urls, v.to_string());
            }
        }
        rest = &rest[gt + 1..];
    }
    (tags, urls)
}

fn diff(source: &Bag, value: &Bag) -> Option<String> {
    source
        .iter()
        .find(|(k, n)| value.get(*k) != Some(n))
        .map(|(k, _)| format!("{k} missing or changed"))
        .or_else(|| {
            value
                .keys()
                .find(|k| !source.contains_key(*k))
                .map(|k| format!("{k} added"))
        })
}

/// `Err` = the translation must not be written (message for the merchant; its fix is `super::fix_code`).
pub fn check(source: &str, value: &str, html: bool) -> Result<(), String> {
    if let Some(d) = diff(&code(source), &code(value)) {
        return Err(format!("placeholder / template code: {d}"));
    }
    if html {
        let ((st, su), (vt, vu)) = (markup(source), markup(value));
        if let Some(d) = diff(&st, &vt) {
            return Err(format!("markup: tag <{d}>"));
        }
        // Localized links are legitimate only when the translator did not invent them: same set of targets.
        if let Some(d) = diff(&su, &vu) {
            return Err(format!("markup: link or media target {d}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn liquid_shortcodes_variables_and_markup() {
        // Liquid output and tags, kept (whitespace inside may move).
        assert!(
            check(
                "Hi {{ customer.first_name }}, {% if x %}yes{% endif %}",
                "Bonjour {{customer.first_name}}, {% if x %}oui{% endif %}",
                false
            )
            .is_ok()
        );
        assert!(
            check(
                "Hi {{ customer.first_name }}!",
                "Bonjour {{ customer.first_name }} !",
                false
            )
            .is_ok()
        );
        assert!(
            check(
                "Hi {{ customer.first_name }}!",
                "Bonjour {{ client.prénom }} !",
                false
            )
            .is_err()
        );
        assert!(check("{% if a %}A{% endif %}", "{% si a %}A{% endif %}", false).is_err());
        // Shortcodes: names and count kept, attributes free.
        assert!(
            check(
                "[button label=\"Buy\"]Now[/button]",
                "[button label=\"Acheter\"]Maintenant[/button]",
                false
            )
            .is_ok()
        );
        assert!(
            check(
                "See [gallery ids=\"1,2\"]",
                "Voir [galerie ids=\"1,2\"]",
                false
            )
            .is_err()
        );
        assert!(check("See [gallery ids=\"1,2\"]", "Voir", false).is_err());
        assert!(
            check(
                "Read the [Note] and [1]",
                "Lire la [Remarque] et [1]",
                false
            )
            .is_ok()
        );
        // Variables.
        assert!(
            check(
                "{count} items for %s, %1$s, ${x}, %{y}",
                "%s : {count} articles, %1$s, ${x}, %{y}",
                false
            )
            .is_ok()
        );
        assert!(check("{count} items", "{nombre} articles", false).is_err());
        assert!(check("100% sure, 50%", "100 % sûr, 50 %", false).is_ok());
        assert!(check("Hello %s", "Bonjour", false).is_err());
        // HTML: tags and targets.
        let src =
            "<p>Soft <b>cotton</b> <a href=\"/a\">shirt</a><img src=\"x.png\" alt=\"A shirt\"></p>";
        assert!(check(src, "<p><a href=\"/a\">Chemise</a> en <b>coton</b> doux<img src=\"x.png\" alt=\"Une chemise\"></p>", true).is_ok());
        assert!(
            check(
                src,
                "<p>Chemise en coton doux <a href=\"/a\">x</a><img src=\"x.png\"></p>",
                true
            )
            .is_err()
        );
        assert!(
            check(
                src,
                "<p>Soft <b>cotton</b> <a href=\"/evil\">shirt</a><img src=\"x.png\"></p>",
                true
            )
            .is_err()
        );
        assert!(check("<p>Hi</p>", "<p>Salut <0>x</0></p>", true).is_err());
        assert!(
            check(
                "<!-- wp:paragraph --><p>Hi</p><!-- /wp:paragraph -->",
                "<!-- wp:paragraph --><p>Salut</p><!-- /wp:paragraph -->",
                true
            )
            .is_ok()
        );
        assert!(check("a < b and c > d", "a < b et c > d", true).is_ok());
    }
}
