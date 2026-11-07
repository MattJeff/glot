//! JS web catalogs read and written through the shared formats: ember-intl / vue-i18n YAML.
use crate::catalog::{Catalog, Format};
use std::path::Path;

#[test]
fn icu_yaml_keeps_plurals_as_strings() {
    // ember-intl `translations/en-us.yaml`: ICU strings, no locale root.
    let src = "# Greetings\nhello: Hello {name}\nphotos: \"{count, plural, =0 {no photos} one {# photo} other {# photos}}\"\nnav:\n  home: Home   # top nav\n";
    let path = Path::new("translations/en-us.yaml");
    let p = super::parse(Format::Yaml, path, src, "en-us").unwrap();
    assert_eq!(
        p.entries["photos"],
        "{count, plural, =0 {no photos} one {# photo} other {# photos}}"
    );
    let cat = Catalog {
        locale: "en-us".into(),
        entries: p.entries.clone(),
    };
    // Byte-identical round trip.
    assert_eq!(super::write(Format::Yaml, path, Some(src), &cat), src);
    // A new plural in a target stays one ICU string (no `one:`/`other:` mapping).
    let mut fr = Catalog::new("fr-fr");
    fr.entries.insert("hello".into(), "Bonjour {name}".into());
    fr.entries.insert(
        "photos".into(),
        "{count, plural, =0 {aucune photo} one {# photo} many {# photos} other {# photos}}".into(),
    );
    let out = super::write(
        Format::Yaml,
        Path::new("translations/fr-fr.yaml"),
        Some("hello: Bonjour {name}\n"),
        &fr,
    );
    let back = super::parse(
        Format::Yaml,
        Path::new("translations/fr-fr.yaml"),
        &out,
        "fr-fr",
    )
    .unwrap();
    assert_eq!(back.entries, fr.entries, "{out}");
    assert!(!out.contains("one:"), "{out}");
}

#[test]
fn rails_yaml_still_writes_plural_mappings() {
    // New Rails file (`.yml`, locale root): CLDR mapping.
    let mut pl = Catalog::new("pl");
    pl.entries.insert(
        "inbox".into(),
        "{count, plural, one {%{count} wiadomość} few {%{count} wiadomości} many {%{count} wiadomości} other {%{count} wiadomości}}".into(),
    );
    let out = super::write(Format::Yaml, Path::new("config/locales/pl.yml"), None, &pl);
    assert!(out.contains("few:"), "{out}");
}

/// `src/{ns}.vue` lists the components with an `<i18n>` block (keys prefixed by their path), and
/// the Angular `messages.{locale}.xtb` source is `messages.xmb`; both read and written end to end.
#[test]
fn vue_components_and_xmb_through_the_config() {
    let dir = std::env::temp_dir().join(format!("glot-jsweb-{}", std::process::id()));
    let w = |p: &str, t: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    w(
        "src/components/nav/Bar.vue",
        "<template/>\n<i18n>\n{\n  \"en\": {\n    \"home\": \"Home\"\n  }\n}\n</i18n>\n",
    );
    w(
        "src/App.vue",
        "<i18n locale=\"en\">\n{ \"t\": \"T\" }\n</i18n>\n",
    );
    w("src/Plain.vue", "<template/>\n");
    w(
        "node_modules/x/Y.vue",
        "<i18n>{\"en\":{\"x\":\"X\"}}</i18n>\n",
    );
    w(
        "src/locale/messages.xmb",
        "<messagebundle>\n  <msg id=\"hi\"><source>a.html:1</source>Hi <ph name=\"INTERPOLATION\"><ex>{{n}}</ex>{{n}}</ph></msg>\n</messagebundle>\n",
    );
    let c: crate::config::Config = toml::from_str(
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"src/{ns}.vue\", \"src/locale/messages.{locale}.xtb\"]\n",
    )
    .unwrap();
    let en = Catalog::load(&c, &dir, "en").unwrap();
    assert_eq!(
        en.entries.into_iter().collect::<Vec<_>>(),
        [
            ("App:t".to_string(), "T".to_string()),
            ("components/nav/Bar:home".into(), "Home".into()),
            ("hi".into(), "Hi {INTERPOLATION}".into()),
        ]
    );
    for (p, layout) in c.catalog_files(&dir, "fr") {
        let mut fr = Catalog::new("fr");
        let key = format!("{}home", layout.ns);
        fr.entries.insert(key, "Accueil".into());
        if layout.ns.is_empty() {
            fr.entries
                .insert("hi".into(), "Salut {INTERPOLATION}".into());
        }
        fr.write_with(&p, Format::Auto, &layout).unwrap();
    }
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    assert!(
        read("src/components/nav/Bar.vue")
            .contains("  },\n  \"fr\": {\n    \"home\": \"Accueil\"\n  }\n}\n</i18n>")
    );
    assert!(
        read("src/App.vue")
            .ends_with("<i18n locale=\"fr\">\n{\n  \"home\": \"Accueil\"\n}\n</i18n>\n")
    );
    assert!(
        read("src/locale/messages.fr.xtb")
            .contains("<translation id=\"hi\">Salut <ph name=\"INTERPOLATION\"/></translation>")
    );
    assert_eq!(read("src/Plain.vue"), "<template/>\n");
    assert_eq!(
        Catalog::load(&c, &dir, "fr").unwrap().entries["App:home"],
        "Accueil"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
