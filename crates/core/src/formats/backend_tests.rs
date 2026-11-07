//! Back-end catalogs as their frameworks ship them: byte-for-byte round trips, one-line diffs
//! when a value changes, the validator on each framework's syntax, gettext `.pot` sources.
use std::path::{Path, PathBuf};

use super::*;
use crate::validate::{IssueCode, Syntax, validate};

const SYMFONY_YAML: &str = "# Shown on the home page\nhello: 'Hello %name%!'\napples: '{0} No apples|{1} One apple|]1,Inf[ %count% apples'\nnav:\n    # top bar\n    home: Home\n    contact: \"Contact us\"\n";

const SYMFONY_XLF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<xliff xmlns="urn:oasis:names:tc:xliff:document:1.2" version="1.2">
  <file source-language="en" target-language="fr" datatype="plaintext" original="file.ext">
    <body>
      <trans-unit id="a1" resname="email.invalid">
        <source>The email {{ value }} is invalid.</source>
        <target>L'adresse {{ value }} est invalide.</target>
      </trans-unit>
      <trans-unit id="a2" resname="too.long">
        <source>Max {{ limit }} characters.</source>
        <target state="needs-translation">Max {{ limit }} characters.</target>
      </trans-unit>
    </body>
  </file>
</xliff>
"#;

const SPRING: &str = "# Greetings\n! legacy comment\ngreeting=Hello {0}, you have {1,number} messages\nit.quote = It''s {0}\nmulti=first \\\n  second\nunicode=Caf\\u00e9\nlabel.save: Save\n\nempty=\n";

const RESX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<root>
  <resheader name="resmimetype">
    <value>text/microsoft-resx</value>
  </resheader>
  <data name="Hello" xml:space="preserve">
    <value>Hello {0}</value>
    <comment>Greeting on the dashboard</comment>
  </data>
  <data name="Price" xml:space="preserve">
    <value>Price: {0:C2}</value>
  </data>
</root>
"#;

const LARAVEL: &str = "<?php\n\nreturn [\n    // Greeting\n    'welcome' => 'Welcome, :name!',\n    'apples' => '{0} No apples|{1} One apple|[2,*] :count apples',\n    'nested' => [\n        'title' => \"It's here\",\n    ],\n];\n";

const PHOENIX_PO: &str = "msgid \"\"\nmsgstr \"\"\n\"Language: fr\\n\"\n\"Plural-Forms: nplurals=2; plural=(n > 1);\\n\"\n\n#: lib/app_web/live/page.ex:3\nmsgid \"Hello %{name}\"\nmsgstr \"Bonjour %{name}\"\n\nmsgid \"One file\"\nmsgid_plural \"%{count} files\"\nmsgstr[0] \"Un fichier\"\nmsgstr[1] \"%{count} fichiers\"\n";

const DEVISE: &str = "# Additional translations at https://github.com/heartcombo/devise/wiki/I18n\nen:\n  devise:\n    failure:\n      locked: \"Your account is locked.\"\n      timeout: Your session expired. Please sign in again to continue.\n    mailer:\n      confirmation_instructions:\n        subject: \"Confirmation instructions\"\n  errors:\n    messages:\n      too_short:\n        one: is too short (minimum is 1 character)\n        other: is too short (minimum is %{count} characters)\n";

const GO_TOML: &str = "# Greetings\nHelloPerson = \"Hello {{.Name}}\"\n\n[PersonCats]\ndescription = \"The number of cats a person has\"\none = \"{{.Name}} has {{.Count}} cat.\"\nother = \"{{.Name}} has {{.Count}} cats.\"\n";

/// (path, content, locale, a key to edit).
fn fixtures() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        (
            "translations/messages.en.yaml",
            SYMFONY_YAML,
            "en",
            "nav.home",
        ),
        (
            "translations/validators.fr.xlf",
            SYMFONY_XLF,
            "fr",
            "email.invalid",
        ),
        (
            "src/main/resources/messages.properties",
            SPRING,
            "en",
            "it.quote",
        ),
        ("Resources/SharedResource.resx", RESX, "en", "Price"),
        ("lang/en/messages.php", LARAVEL, "en", "messages.welcome"),
        (
            "priv/gettext/fr/LC_MESSAGES/default.po",
            PHOENIX_PO,
            "fr",
            "Hello %{name}",
        ),
        (
            "config/locales/devise.en.yml",
            DEVISE,
            "en",
            "devise.failure.timeout",
        ),
        ("active.en.toml", GO_TOML, "en", "HelloPerson"),
    ]
}

fn load(path: &str, text: &str, locale: &str) -> (Format, Catalog) {
    let f = native(Format::Auto, Path::new(path)).unwrap();
    let p = parse(f, Path::new(path), text, locale).unwrap_or_else(|e| panic!("{path}: {e}"));
    let cat = Catalog {
        locale: locale.into(),
        entries: p.entries,
    };
    (f, cat)
}

#[test]
fn round_trip_and_one_line_edits() {
    for (path, text, locale, key) in fixtures() {
        let (f, mut cat) = load(path, text, locale);
        assert!(
            cat.entries.contains_key(key),
            "{path}: {:?}",
            cat.entries.keys()
        );
        assert_eq!(write(f, Path::new(path), Some(text), &cat), text, "{path}");
        let new = "Changed: it's %s {{ x }} \"q\"".to_string();
        cat.entries.insert(key.into(), new.clone());
        let out = write(f, Path::new(path), Some(text), &cat);
        let changed = text
            .lines()
            .zip(out.lines())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(changed, 1, "{path}:\n{out}");
        assert_eq!(text.lines().count(), out.lines().count(), "{path}:\n{out}");
        let (_, again) = load(path, &out, locale);
        assert_eq!(again.entries[key], new, "{path}:\n{out}");
    }
}

#[test]
fn values_starting_with_syntax_stay_valid() {
    // `%name%` / `{{ x }}` / `{0}` first: YAML needs quotes, the rest of the file is untouched.
    for v in [
        "%name% is here",
        "{{ count }} items",
        "{0} none|{1} one",
        "@user",
        "- dash",
    ] {
        for (path, text, locale, key) in [
            (
                "translations/messages.en.yaml",
                SYMFONY_YAML,
                "en",
                "nav.home",
            ),
            (
                "config/locales/devise.en.yml",
                DEVISE,
                "en",
                "devise.failure.timeout",
            ),
        ] {
            let (f, mut cat) = load(path, text, locale);
            cat.entries.insert(key.into(), v.into());
            let out = write(f, Path::new(path), Some(text), &cat);
            let (_, again) = load(path, &out, locale);
            assert_eq!(again.entries[key], v, "{path}:\n{out}");
        }
    }
}

fn codes(src: &str, tgt: &str, locale: &str, syntax: Syntax) -> Vec<IssueCode> {
    validate(src, tgt, locale, syntax, &Default::default())
        .into_iter()
        .filter(|i| i.severity == crate::validate::Severity::Error)
        .map(|i| i.code)
        .collect()
}

#[test]
fn framework_placeholders() {
    // Phoenix / Elixir gettext `%{name}` under the gettext syntax.
    assert!(codes("Hello %{name}", "Bonjour %{name}", "fr", Syntax::Printf).is_empty());
    assert_eq!(
        codes("Hello %{name}", "Bonjour %{nom}", "fr", Syntax::Printf),
        [IssueCode::PlaceholderMissing, IssueCode::PlaceholderExtra]
    );
    // Django `python-brace-format` and `python-format`.
    assert!(codes("Welcome {user}", "Bienvenue {user}", "fr", Syntax::Printf).is_empty());
    assert!(!codes("Hi %(name)s", "Salut %(nom)s", "fr", Syntax::Printf).is_empty());
    // .NET composite format: `{0:C2}`, `{{` literal.
    assert!(
        codes(
            "Price: {0:C2} {{x}}",
            "Prix : {0:C2} {{x}}",
            "fr",
            Syntax::Printf
        )
        .is_empty()
    );
    assert!(!codes("Price: {0:C2}", "Prix : {1:C2}", "fr", Syntax::Printf).is_empty());
    // go-i18n templates.
    assert!(
        !codes(
            "Hello {{.Name}}",
            "Bonjour {{.Nom}}",
            "fr",
            Syntax::Template
        )
        .is_empty()
    );
    // Spring: an elided `l'` breaks MessageFormat; `''` is right.
    assert!(!codes("The user {0}", "L'utilisateur {0}", "fr", Syntax::Java).is_empty());
    assert!(codes("The user {0}", "L''utilisateur {0}", "fr", Syntax::Java).is_empty());
    // Symfony validators and legacy plurals.
    assert!(
        !codes(
            "Max {{ limit }}.",
            "Max {{ limite }}.",
            "fr",
            Syntax::Symfony
        )
        .is_empty()
    );
    let src = "{0} No apples|{1} One apple|]1,Inf[ %count% apples";
    assert!(
        codes(
            src,
            "{0} Aucune|{1} Une pomme|]1,Inf[ %count% pommes",
            "fr",
            Syntax::Symfony
        )
        .is_empty()
    );
}

fn tmp(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("glot-backend-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for (p, c) in files {
        let p = d.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
    d
}

const POT: &str = "msgid \"\"\nmsgstr \"\"\n\n#. A button\n#: lib/app_web/live/page.ex:9\nmsgid \"Save %{thing}\"\nmsgstr \"\"\n\nmsgid \"One file\"\nmsgid_plural \"%{count} files\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n";

#[test]
fn gettext_pot_is_the_source_and_the_template() {
    let d = tmp(
        "phx",
        &[
            ("priv/gettext/default.pot", POT),
            ("app/translations/fr/LC_MESSAGES/messages.po", ""),
            ("messages.pot", POT),
        ],
    );
    let c: Config = toml::from_str(
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"priv/gettext/{locale}/LC_MESSAGES/default.po\", \"app/translations/{locale}/LC_MESSAGES/messages.po\"]\n",
    )
    .unwrap();
    // Phoenix `priv/gettext/default.pot`, pybabel's `messages.pot` at the project root.
    assert_eq!(
        c.catalog_paths(&d, "en"),
        [d.join("priv/gettext/default.pot"), d.join("messages.pot")]
    );
    // A source `.po` wins over the template.
    std::fs::create_dir_all(d.join("priv/gettext/en/LC_MESSAGES")).unwrap();
    std::fs::write(d.join("priv/gettext/en/LC_MESSAGES/default.po"), "").unwrap();
    assert_eq!(
        c.catalog_paths(&d, "en")[0],
        d.join("priv/gettext/en/LC_MESSAGES/default.po")
    );
    // New target entries copy the template's comments, references and `msgid_plural`.
    let target = d.join("priv/gettext/fr/LC_MESSAGES/default.po");
    let mut cat = Catalog::new("fr");
    cat.entries
        .insert("Save %{thing}".into(), "Enregistrer %{thing}".into());
    cat.entries.insert(
        "One file".into(),
        "{count, plural, one {Un fichier} other {%{count} fichiers}}".into(),
    );
    write_path(Format::Po, &target, &cat).unwrap();
    let out = std::fs::read_to_string(&target).unwrap();
    assert!(
        out.contains("#. A button\n#: lib/app_web/live/page.ex:9\nmsgid \"Save %{thing}\"\nmsgstr \"Enregistrer %{thing}\"\n"),
        "{out}"
    );
    assert!(out.contains("msgid_plural \"%{count} files\"\n"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}
