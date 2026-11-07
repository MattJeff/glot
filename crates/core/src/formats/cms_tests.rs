//! Static sites / CMS catalogs: WordPress `.pot` as source, Shopify `en.default.json` + banner.
use std::path::PathBuf;

use crate::catalog::{Catalog, Format};
use crate::config::Config;

fn tmp(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("glot-cms-core-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for (p, c) in files {
        let p = d.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
    d
}

fn config(files: &str) -> Config {
    toml::from_str(&format!(
        "source_locale = \"en\"\ntarget_locales = [\"fr_FR\"]\nfiles = [{files}]\n"
    ))
    .unwrap()
}

const POT: &str = "msgid \"\"\nmsgstr \"\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\n#: my-plugin.php:8\n#, php-format\nmsgid \"%s item\"\nmsgid_plural \"%s items\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n";

#[test]
fn wordpress_pot_is_the_source_and_the_template() {
    let d = tmp(
        "wp",
        &[
            ("languages/my-plugin.pot", POT),
            ("theme/languages/twenty.pot", POT),
        ],
    );
    let c = config("\"languages/my-plugin-{locale}.po\"");
    assert_eq!(
        c.catalog_paths(&d, "en"),
        [d.join("languages/my-plugin.pot")]
    );
    assert_eq!(
        c.catalog_paths(&d, "fr_FR"),
        [d.join("languages/my-plugin-fr_FR.po")]
    );
    let t = config("\"theme/languages/{locale}.po\"");
    assert_eq!(
        t.catalog_paths(&d, "en"),
        [d.join("theme/languages/twenty.pot")]
    );

    let src = Catalog::load(&c, &d, "en").unwrap();
    let mut meta = src.entries.clone();
    let mut lock = crate::lock::Lockfile::default();
    let mut cats = std::collections::BTreeMap::from([("en".to_string(), src)]);
    super::load_meta(&c, &d, &mut cats, &mut lock).unwrap();
    meta.extend(cats["en"].entries.clone());
    assert_eq!(
        meta["%s item"],
        "{count, plural, one {%s item} other {%s items}}"
    );

    // A new target entry keeps the template's references, flags and msgid_plural.
    let fr = Catalog {
        locale: "fr_FR".into(),
        entries: [(
            "%s item".to_string(),
            "{count, plural, one {%s article} other {%s articles}}".to_string(),
        )]
        .into(),
    };
    let path = d.join("languages/my-plugin-fr_FR.po");
    fr.write(&path, Format::Auto).unwrap();
    let out = std::fs::read_to_string(&path).unwrap();
    assert!(
        out.contains("#: my-plugin.php:8\n#, php-format\nmsgid \"%s item\"\nmsgid_plural \"%s items\"\nmsgstr[0] \"%s article\"\nmsgstr[1] \"%s articles\"\n"),
        "{out}"
    );
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn shopify_default_source_and_banner() {
    let banner = "/*\n * IMPORTANT: auto-generated.\n */\n";
    let d = tmp(
        "shopify",
        &[
            (
                "locales/en.default.json",
                &format!("{banner}{{\n  \"a\": {{ \"b\": \"Hi {{{{ name }}}}\" }}\n}}\n"),
            ),
            ("locales/en.default.schema.json", "{ \"s\": \"Colors\" }"),
            (
                "locales/fr.json",
                &format!("{banner}{{\n  \"a\": {{\n    \"b\": \"Salut\"\n  }}\n}}\n"),
            ),
        ],
    );
    let c = config("\"locales/{locale}.json\", \"locales/{locale}.schema.json\"");
    assert_eq!(
        c.catalog_paths(&d, "en"),
        [
            d.join("locales/en.default.json"),
            d.join("locales/en.default.schema.json")
        ]
    );
    assert_eq!(
        c.catalog_paths(&d, "fr"),
        [d.join("locales/fr.json"), d.join("locales/fr.schema.json")]
    );
    let en = Catalog::load(&c, &d, "en").unwrap();
    assert_eq!(en.entries["a.b"], "Hi {{ name }}");
    assert_eq!(en.entries["s"], "Colors");
    let fr_path = d.join("locales/fr.json");
    let mut fr = Catalog::read(&fr_path, "fr", Format::Auto).unwrap();
    fr.entries.insert("a.b".into(), "Salut {{ name }}".into());
    fr.write(&fr_path, Format::Auto).unwrap();
    let out = std::fs::read_to_string(&fr_path).unwrap();
    assert!(out.starts_with(banner), "{out}");
    assert!(out.contains("\"b\": \"Salut {{ name }}\""), "{out}");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn hugo_yaml_new_target_gets_plural_mappings() {
    let d = tmp(
        "hugo-yaml",
        &[("hugo.toml", "baseURL = 'https://x.org/'\n")],
    );
    let pl = Catalog {
        locale: "pl".into(),
        entries: [(
            "wordCount".to_string(),
            "{count, plural, one {{{ .Count }} słowo} few {{{ .Count }} słowa} many {{{ .Count }} słów} other {{{ .Count }} słowa}}".to_string(),
        )]
        .into(),
    };
    let path = d.join("i18n/pl.yaml");
    pl.write(&path, Format::Auto).unwrap();
    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.starts_with("wordCount:\n  one: "), "{out}");
    assert_eq!(
        Catalog::read(&path, "pl", Format::Auto).unwrap().entries,
        pl.entries
    );
    std::fs::remove_dir_all(d).unwrap();
}
