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
