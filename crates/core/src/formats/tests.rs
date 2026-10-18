use std::path::Path;

use super::*;
use crate::validate::{IssueCode, Syntax, validate};

macro_rules! fixture {
    ($f:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/formats/",
            $f
        ))
    };
}

/// (path, content, locale) for every fixture.
fn all() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "Localizable.xcstrings",
            fixture!("Localizable.xcstrings"),
            "en",
        ),
        (
            "Localizable.xcstrings",
            fixture!("Localizable.xcstrings"),
            "fr",
        ),
        (
            "en.lproj/Localizable.strings",
            fixture!("Localizable.strings"),
            "en",
        ),
        (
            "en.lproj/Localizable.stringsdict",
            fixture!("Localizable.stringsdict"),
            "en",
        ),
        ("res/values/strings.xml", fixture!("strings.xml"), "en"),
        ("l10n/app_en.arb", fixture!("app_en.arb"), "en"),
        ("locale/fr/LC_MESSAGES/django.po", fixture!("fr.po"), "fr"),
        ("config/locales/en.yml", fixture!("en.yml"), "en"),
    ]
}

fn load(path: &str, text: &str, locale: &str) -> (Format, Parsed, Catalog) {
    let f = native(Format::Auto, Path::new(path)).unwrap();
    let p = parse(f, Path::new(path), text, locale).unwrap();
    let cat = Catalog {
        locale: locale.into(),
        entries: p.entries.clone(),
    };
    (f, p, cat)
}

#[test]
fn byte_for_byte_roundtrip() {
    for (path, text, locale) in all() {
        let (f, _, cat) = load(path, text, locale);
        assert_eq!(
            write(f, Path::new(path), Some(text), &cat),
            text,
            "{path} ({locale})"
        );
    }
}

#[test]
fn parsed_values() {
    let (_, en, _) = load("L.xcstrings", fixture!("Localizable.xcstrings"), "en");
    let e = &en.entries;
    assert_eq!(
        e["%lld items"],
        "{count, plural, one {%lld item} other {%lld items}}"
    );
    assert_eq!(e["Save"], "Save");
    assert_eq!(e["Hello, %@!"], "Hello, %@!");
    assert!(!e.contains_key("Acme"), "shouldTranslate=false is skipped");
    assert_eq!(
        e["Tap to continue"],
        "{device, select, mac {Click to continue} other {Tap to continue}}"
    );
    assert_eq!(
        e["You have %lld new messages from %@"],
        "You have {messages, plural, one {%arg new message} other {%arg new messages}} from %@"
    );
    assert_eq!(en.notes["Hello, %@!"], "Greeting on the home screen");
    let (_, fr, _) = load("L.xcstrings", fixture!("Localizable.xcstrings"), "fr");
    assert_eq!(fr.entries.len(), 2);
    assert!(fr.stale.contains("Hello, %@!"));

    let (_, s, _) = load("a.strings", fixture!("Localizable.strings"), "en");
    assert_eq!(s.entries["quote"], "She said \"hi\"\nthen left");
    assert_eq!(
        s.notes["welcome.greeting"],
        "Greeting, %@ is the user's first name"
    );
    let (_, d, _) = load("a.stringsdict", fixture!("Localizable.stringsdict"), "en");
    assert_eq!(
        d.entries["files.count"],
        "{files, plural, one {%d file} other {%d files}}"
    );
    assert_eq!(
        d.entries["inbox.summary"],
        "%2$@ has {unread, plural, one {%ld unread message} other {%ld unread messages}}"
    );

    let (_, a, _) = load("strings.xml", fixture!("strings.xml"), "en");
    assert!(!a.entries.contains_key("app_name"));
    assert_eq!(a.entries["dont_worry"], "Don't worry, it's saved.");
    assert_eq!(a.entries["terms"], "Read our <b>terms</b> & conditions");
    assert_eq!(a.entries["tos"], "Terms & Privacy");
    assert_eq!(
        a.entries["songs_found"],
        "{count, plural, one {%d song found.} other {%d songs found.}}"
    );
    assert_eq!(a.entries["planets[1]"], "Venus");
    assert_eq!(a.notes["login_title"], "Shown on the login screen");

    let (_, arb, _) = load("app_en.arb", fixture!("app_en.arb"), "en");
    assert_eq!(arb.entries.len(), 3);
    assert_eq!(
        arb.notes["greeting"],
        "Greeting on the profile page (placeholders: {userName}: String, e.g. Bob)"
    );

    let (_, po, _) = load("fr.po", fixture!("fr.po"), "fr");
    assert_eq!(po.entries["Dashboard"], "Tableau de bord");
    assert_eq!(po.entries["Welcome, %(name)s!"], "");
    assert_eq!(po.implicit["Welcome, %(name)s!"], "Welcome, %(name)s!");
    let pl = "You have %(count)s unread message";
    assert_eq!(
        po.entries[pl],
        "{count, plural, one {Vous avez %(count)s message} other {Vous avez %(count)s messages}}"
    );
    assert!(po.stale.contains(pl));
    assert_eq!(po.entries["verb\u{4}Open"], "Ouvrir");
    assert_eq!(
        po.entries["A long text that spans several lines.\n"],
        "Un long texte qui s'étend sur plusieurs lignes.\n"
    );
    assert!(!po.entries.contains_key("Old"));
    assert_eq!(
        po.notes["Dashboard"],
        "Title of the dashboard page\nReferences: templates/dashboard.html:12"
    );

    let (_, y, _) = load("en.yml", fixture!("en.yml"), "en");
    assert_eq!(y.entries["nav.home"], "Home");
    assert_eq!(y.entries["greeting"], "Hello %{name}!");
    assert_eq!(
        y.entries["inbox.messages"],
        "{count, plural, one {%{count} message} other {%{count} messages}}"
    );
    assert_eq!(y.entries["quote"], "It's fine");
    assert_eq!(y.entries["legal"], "Line one.\nLine two.\n");
    assert_eq!(y.notes["greeting"], "Shown after login");
    assert!(!y.entries.contains_key("flags"));
}

/// Change one value, drop one key, add a plain and a plural key: re-read gives the catalog back,
/// everything else in the file is untouched.
#[test]
fn edit_remove_add() {
    let cases: &[(&str, &str, &str, &str, &str, &str)] = &[
        // path, text, locale, key to change, key to drop, kept text
        (
            "en.lproj/Localizable.strings",
            fixture!("Localizable.strings"),
            "en",
            "welcome.title",
            "action.save",
            "/* Greeting, %@ is the user's first name */",
        ),
        (
            "en.lproj/Localizable.stringsdict",
            fixture!("Localizable.stringsdict"),
            "en",
            "inbox.summary",
            "files.count",
            "<!DOCTYPE plist",
        ),
        (
            "res/values/strings.xml",
            fixture!("strings.xml"),
            "en",
            "welcome",
            "tos",
            "tools:ignore=\"TypographyApostrophe\"",
        ),
        (
            "l10n/app_en.arb",
            fixture!("app_en.arb"),
            "en",
            "greeting",
            "helloWorld",
            "\"type\": \"String\"",
        ),
        (
            "locale/fr/LC_MESSAGES/django.po",
            fixture!("fr.po"),
            "fr",
            "Dashboard",
            "verb\u{4}Open",
            "#: templates/dashboard.html:12",
        ),
        (
            "config/locales/en.yml",
            fixture!("en.yml"),
            "en",
            "nav.settings",
            "quote",
            "# Navigation",
        ),
        (
            "Localizable.xcstrings",
            fixture!("Localizable.xcstrings"),
            "fr",
            "Hello, %@!",
            "%lld items",
            "\"shouldTranslate\" : false",
        ),
    ];
    for (path, text, locale, change, drop, kept) in cases {
        let (f, _, mut cat) = load(path, text, locale);
        cat.entries
            .insert(change.to_string(), "Changed \"quoted\" & 'new' %@".into());
        cat.entries.remove(*drop);
        cat.entries.insert("brand.new".into(), "Brand new".into());
        cat.entries.insert(
            "brand.count".into(),
            "{count, plural, one {%d thing} other {%d things}}".into(),
        );
        let out = write(f, Path::new(path), Some(text), &cat);
        assert!(out.contains(kept), "{path}: lost {kept:?}\n{out}");
        let back = parse(f, Path::new(path), &out, locale).unwrap();
        assert_eq!(back.entries, cat.entries, "{path}\n{out}");
        // Writing again is stable.
        assert_eq!(write(f, Path::new(path), Some(&out), &cat), out, "{path}");
    }
}

#[test]
fn new_files_and_plural_targets() {
    // Target files that don't exist yet, with the target locale's CLDR categories.
    let ru = "{count, plural, one {%d файл} few {%d файла} many {%d файлов} other {%d файла}}";
    for (path, locale) in [
        ("res/values-ru/strings.xml", "ru"),
        ("ru.lproj/Localizable.stringsdict", "ru"),
        ("l10n/app_ru.arb", "ru"),
        ("locale/ru/LC_MESSAGES/django.po", "ru"),
        ("config/locales/ru.yml", "ru"),
        ("Localizable.xcstrings", "ru"),
        ("ru.lproj/Localizable.strings", "ru"),
    ] {
        let f = native(Format::Auto, Path::new(path)).unwrap();
        let mut cat = Catalog::new(locale);
        cat.entries.insert("files".into(), ru.into());
        cat.entries.insert("nav.title".into(), "Главная".into());
        let out = write(f, Path::new(path), None, &cat);
        let back = parse(f, Path::new(path), &out, locale).unwrap();
        let expected = if path.ends_with(".po") {
            // gettext ru has 3 forms: CLDR `other` (fractions) is not representable.
            "{count, plural, one {%d файл} few {%d файла} many {%d файлов}}"
        } else {
            ru
        };
        assert_eq!(back.entries["files"], expected, "{path}\n{out}");
        assert_eq!(back.entries["nav.title"], "Главная", "{path}");
    }
    let yml = write(Format::Yaml, Path::new("config/locales/ru.yml"), None, &{
        let mut c = Catalog::new("ru");
        c.entries.insert("nav.title".into(), "Главная".into());
        c
    });
    assert_eq!(yml, "ru:\n  nav:\n    title: Главная\n");
}

#[test]
fn fuzzy_cleared_on_write() {
    let (f, _, mut cat) = load("fr.po", fixture!("fr.po"), "fr");
    let pl = "You have %(count)s unread message";
    cat.entries.insert(
        pl.into(),
        "{count, plural, one {Vous avez %(count)s message non lu} other {Vous avez %(count)s messages non lus}}".into(),
    );
    cat.entries
        .insert("Welcome, %(name)s!".into(), "Bienvenue, %(name)s !".into());
    let out = write(f, Path::new("fr.po"), Some(fixture!("fr.po")), &cat);
    assert!(out.contains("#, python-format\nmsgid \"You have"), "{out}");
    assert!(out.contains("msgstr[1] \"Vous avez %(count)s messages non lus\""));
    assert!(out.contains("msgstr \"Bienvenue, %(name)s !\""));
    assert!(
        !parse(f, Path::new("fr.po"), &out, "fr")
            .unwrap()
            .stale
            .contains(pl)
    );
    // xcstrings: a rewritten value is `translated`.
    let (f, _, mut cat) = load("L.xcstrings", fixture!("Localizable.xcstrings"), "fr");
    cat.entries
        .insert("Hello, %@!".into(), "Salut, %@ !".into());
    let out = write(
        f,
        Path::new("L.xcstrings"),
        Some(fixture!("Localizable.xcstrings")),
        &cat,
    );
    assert!(
        parse(f, Path::new("L.xcstrings"), &out, "fr")
            .unwrap()
            .stale
            .is_empty()
    );
}

#[test]
fn printf_and_rails_validation() {
    let codes = |s: &str, t: &str, loc: &str, syn| -> Vec<IssueCode> {
        validate(s, t, loc, syn, &Default::default())
            .into_iter()
            .filter(|i| i.severity == crate::validate::Severity::Error)
            .map(|i| i.code)
            .collect()
    };
    use IssueCode::*;
    let p = Syntax::Printf;
    assert_eq!(codes("Hello, %@!", "Bonjour, %@ !", "fr", p), []);
    assert_eq!(
        codes("Hello, %@!", "Bonjour !", "fr", p),
        [PlaceholderMissing]
    );
    assert_eq!(codes("Hi", "Salut %@", "fr", p), [PlaceholderExtra]);
    // Positional reordering is fine, conversions must match.
    assert_eq!(
        codes(
            "%1$s sent %2$d files",
            "%2$d fichiers envoyés par %1$s",
            "fr",
            p
        ),
        []
    );
    assert_eq!(codes("%@ and %@", "%2$@ et %1$@", "fr", p), []);
    assert_eq!(
        codes("%d files", "%@ fichiers", "fr", p),
        [KeywordTranslated]
    );
    assert_eq!(
        codes("Welcome, %(name)s!", "Bienvenue, %(nom)s !", "fr", p),
        [PlaceholderMissing, PlaceholderExtra]
    );
    assert_eq!(
        codes("100%% done, 50% off", "100%% fait, 50 % de remise", "fr", p),
        []
    );
    // Plural wrapper: CLDR categories of the target locale.
    let src = "{count, plural, one {%d item} other {%d items}}";
    assert_eq!(
        codes(
            src,
            "{count, plural, one {%d élément} many {%d éléments} other {%d éléments}}",
            "fr",
            p
        ),
        []
    );
    assert_eq!(
        codes(
            src,
            "{count, plural, one {%d элемент} other {%d элементов}}",
            "ru",
            p
        ),
        [PluralCategoryMissing]
    );
    assert_eq!(
        codes(
            src,
            "{count, plural, one {élément} many {%d éléments} other {%d éléments}}",
            "fr",
            p
        ),
        []
    );
    // Rails `%{name}`.
    let r = Syntax::Rails;
    assert_eq!(codes("Hello %{name}!", "Bonjour %{name} !", "fr", r), []);
    assert_eq!(
        codes("Hello %{name}!", "Bonjour %{nom} !", "fr", r),
        [PlaceholderMissing, PlaceholderExtra]
    );
    let src = "{count, plural, one {%{count} message} other {%{count} messages}}";
    assert_eq!(
        codes(
            src,
            "{count, plural, one {%{count} message} many {%{count} messages} other {%{count} messages}}",
            "fr",
            r
        ),
        []
    );
    assert_eq!(
        codes(src, "{count, plural, other {%{count} messages}}", "fr", r),
        [PluralCategoryMissing]
    );
    // ARB is plain ICU.
    assert_eq!(
        codes("Hello {userName}", "Bonjour {userName}", "fr", Syntax::Icu),
        []
    );
}

#[test]
fn android_qualifiers() {
    use crate::config::android_qualifier as q;
    assert_eq!(
        [
            q("fr"),
            q("pt-BR"),
            q("sr-Latn"),
            q("es-419"),
            q("zh-Hant-TW")
        ],
        ["fr", "pt-rBR", "b+sr+Latn", "es-r419", "b+zh+Hant+TW"]
    );
}
