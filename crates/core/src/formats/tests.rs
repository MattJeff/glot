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

/// Field test on real projects: `GLOT_ROUNDTRIP_DIR=/path cargo test -p glot-core roundtrip_dir -- --ignored`.
/// Every native catalog under the dir must be rewritten byte for byte (encoding included) and an
/// edited value must read back.
#[test]
#[ignore]
fn roundtrip_dir() {
    let Ok(dir) = std::env::var("GLOT_ROUNDTRIP_DIR") else {
        return;
    };
    fn walk(d: &Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if native(Format::Auto, &p).is_some()
                && !p.to_string_lossy().contains(".glot")
                && !p
                    .extension()
                    .is_some_and(|x| x == "xml" || x == "yml" || x == "yaml")
            {
                out.push(p);
            }
        }
    }
    let mut files = vec![];
    walk(Path::new(&dir), &mut files);
    let mut failed = vec![];
    for p in &files {
        let bytes = std::fs::read(p).unwrap();
        let (text, enc) = match decode(&bytes) {
            Ok(t) => t,
            Err(e) => {
                failed.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        let f = native(Format::Auto, p).unwrap();
        let parsed = match parse(f, p, &text, "fr") {
            Ok(x) => x,
            Err(e) => {
                failed.push(format!("{}: parse: {e}", p.display()));
                continue;
            }
        };
        let mut cat = Catalog {
            locale: "fr".into(),
            entries: parsed.entries.clone(),
        };
        if encode(&write(f, p, Some(&text), &cat), enc) != bytes {
            failed.push(format!("{}: not byte for byte", p.display()));
            continue;
        }
        if let Some((k, _)) = parsed.entries.iter().find(|(_, v)| !v.contains('{')) {
            cat.entries.insert(k.clone(), "x \"q\" \\ y".into());
            let again = parse(f, p, &write(f, p, Some(&text), &cat), "fr").unwrap();
            if again.entries != cat.entries {
                failed.push(format!("{}: edit does not read back", p.display()));
            }
        }
    }
    eprintln!("{} files checked", files.len());
    assert!(failed.is_empty(), "{failed:#?}");
}

fn js_cat(locale: &str, entries: &[(&str, &str)]) -> Catalog {
    Catalog {
        locale: locale.into(),
        entries: entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn js_dictionary_nested_round_trip() {
    let src = r#"import { HomeIcon } from "icons";
import type { Dict } from "./types";

type Shape = { a: string };

export const enUS: Dict = {
  // Common
  common: {
    home: "Home",
    save: 'Save',
    count: (n: number) => `${n} items`,
    icon: HomeIcon,
  },
  'nav.more': "It's \"more\"\n",
  tpl: `Hello`,
  joined: "a" + "b",
};
"#;
    let p = parse(Format::Js, Path::new("locales/en-US.ts"), src, "en-US").unwrap();
    let got: Vec<(&str, &str)> = p
        .entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("common.home", "Home"),
            ("common.save", "Save"),
            ("nav.more", "It's \"more\"\n"),
            ("tpl", "Hello")
        ]
    );
    // Unchanged catalog => byte-identical file.
    let same = js_cat("en-US", &got);
    assert_eq!(write(Format::Js, Path::new("x.ts"), Some(src), &same), src);

    // Changed value re-quoted in place, removed key dropped (line gone), new keys nested in place.
    let cat = js_cat(
        "en-US",
        &[
            ("common.home", "Start"),
            ("nav.more", "It's \"more\"\n"),
            ("tpl", "Hello"),
            ("common.cancel", "Cancel"),
            ("auth.title", "Sign in"),
        ],
    );
    let out = write(Format::Js, Path::new("x.ts"), Some(src), &cat);
    assert!(out.contains("    home: \"Start\",\n    count:"), "{out}");
    assert!(!out.contains("save"), "{out}");
    assert!(
        out.contains("    icon: HomeIcon,\n    cancel: \"Cancel\",\n  },"),
        "{out}"
    );
    // The root mixes in a dotted key, so it is a flat object: the new key stays flat.
    assert!(
        out.contains("  joined: \"a\" + \"b\",\n  \"auth.title\": \"Sign in\",\n};"),
        "{out}"
    );
    let back = parse(Format::Js, Path::new("x.ts"), &out, "en-US").unwrap();
    assert_eq!(back.entries, cat.entries);
}

#[test]
fn js_flat_dictionary_and_new_file() {
    let src = "const it: Partial<T> = {\n  \"nav.home\": \"Home\",\n  \"nav.map\": \"Map\"\n};\nexport default it;\n";
    let cat = js_cat(
        "it",
        &[
            ("nav.home", "Inizio"),
            ("nav.map", "Mappa"),
            ("nav.saved", "Salvati"),
        ],
    );
    let out = write(Format::Js, Path::new("it.ts"), Some(src), &cat);
    assert_eq!(
        out,
        "const it: Partial<T> = {\n  \"nav.home\": \"Inizio\",\n  \"nav.map\": \"Mappa\",\n  \"nav.saved\": \"Salvati\",\n};\nexport default it;\n"
    );
    let fresh = write(
        Format::Js,
        Path::new("de.ts"),
        None,
        &js_cat("de", &[("a.b", "x")]),
    );
    assert!(
        fresh.starts_with("export default {") && fresh.ends_with("};\n"),
        "{fresh}"
    );
    assert_eq!(
        parse(Format::Js, Path::new("de.ts"), &fresh, "de")
            .unwrap()
            .entries["a.b"],
        "x"
    );
}

#[test]
fn js_inline_i18next_resources() {
    let src = "import i18n from 'i18next';\n\nconst resources = {\n  en: {\n    translation: {\n      hero: { title: 'Hi', cta: 'Go' },\n    },\n  },\n  fr: {\n    translation: {\n      hero: { title: 'Salut' },\n    },\n  },\n};\n\ni18n.init({ resources, lng: 'en' });\n";
    let en = parse(Format::Js, Path::new("src/i18n.js"), src, "en").unwrap();
    assert_eq!(en.entries.len(), 2);
    assert_eq!(en.entries["hero.cta"], "Go");
    let fr = parse(Format::Js, Path::new("src/i18n.js"), src, "fr").unwrap();
    assert_eq!(fr.entries["hero.title"], "Salut");
    // fr gains a key; a new locale is appended with the same `translation` wrapper.
    let out = write(
        Format::Js,
        Path::new("src/i18n.js"),
        Some(src),
        &js_cat("fr", &[("hero.title", "Salut"), ("hero.cta", "Vas-y")]),
    );
    assert!(
        out.contains("hero: { title: 'Salut', cta: 'Vas-y' }") || out.contains("cta: 'Vas-y',"),
        "{out}"
    );
    assert_eq!(
        parse(Format::Js, Path::new("x.js"), &out, "fr")
            .unwrap()
            .entries["hero.cta"],
        "Vas-y"
    );
    assert_eq!(
        parse(Format::Js, Path::new("x.js"), &out, "en")
            .unwrap()
            .entries,
        en.entries
    );
    let out = write(
        Format::Js,
        Path::new("src/i18n.js"),
        Some(src),
        &js_cat("de", &[("hero.title", "Hallo")]),
    );
    let de = parse(Format::Js, Path::new("x.js"), &out, "de").unwrap();
    assert_eq!(de.entries["hero.title"], "Hallo");
    assert!(
        out.contains("  de: {\n    translation: {\n      hero: {\n        title: 'Hallo',"),
        "{out}"
    );
}

/// AI-builder pattern (anonymized): the dictionary lives in `LanguageContext.tsx` next to JSX,
/// generics and a `type Language` union.
#[test]
fn js_dictionary_inside_tsx_context() {
    let src = r#"import React, { createContext, useContext, useState } from 'react';
type Language = 'en' | 'fr';
export const languages: Language[] = ['en', 'fr'];
const defaults = { theme: 'dark' };
const translations: Record<Language, Record<string, string>> = {
  en: { 'nav.home': 'Home', 'hero.cta': 'Go', 'hero.title': "Don't wait" },
  fr: { 'nav.home': 'Accueil', 'hero.cta': 'Allez', 'hero.title': "N'attendez pas" },
};
const Ctx = createContext<{ t: (k: string) => string } | undefined>(undefined);
export const LanguageProvider = ({ children }: { children: React.ReactNode }) => {
  const [language] = useState<Language>(() => (localStorage.getItem('lang') as Language) || 'en');
  const t = (key: string) => translations[language][key] || key;
  return <Ctx.Provider value={{ t }}>{children}</Ctx.Provider>;
};
export const useLanguage = () => useContext(Ctx)!;
"#;
    let path = Path::new("src/contexts/LanguageContext.tsx");
    assert_eq!(native(Format::Auto, path), Some(Format::Js));
    let en = parse(Format::Js, path, src, "en").unwrap();
    assert_eq!(en.entries["hero.title"], "Don't wait");
    assert_eq!(super::js::dictionary_locales(src), ["en", "fr"]);
    // Round trip: byte for byte.
    let fr = parse(Format::Js, path, src, "fr").unwrap();
    let same = Catalog {
        locale: "fr".into(),
        entries: fr.entries.clone(),
    };
    assert_eq!(write(Format::Js, path, Some(src), &same), src);
    // New locale: appended block, `type Language` union and the language list follow.
    let out = write(
        Format::Js,
        path,
        Some(src),
        &js_cat("de", &[("nav.home", "Start")]),
    );
    assert!(out.contains("type Language = 'en' | 'fr' | 'de';"), "{out}");
    assert!(out.contains("['en', 'fr', 'de']"), "{out}");
    assert!(
        out.contains("  de: {\n    'nav.home': 'Start',\n  },\n};"),
        "{out}"
    );
    assert!(out.contains("const defaults = { theme: 'dark' };"), "{out}");
    assert_eq!(
        parse(Format::Js, path, &out, "de").unwrap().entries["nav.home"],
        "Start"
    );
    assert_eq!(
        parse(Format::Js, path, &out, "fr").unwrap().entries,
        fr.entries
    );
    assert_parses_tsx(&out);
    // A one-line locale object gains its key on the same line.
    let mut fr2 = fr.entries.clone();
    fr2.insert("hero.sub".into(), "Sans carte".into());
    let out = write(
        Format::Js,
        path,
        Some(src),
        &Catalog {
            locale: "fr".into(),
            entries: fr2.clone(),
        },
    );
    assert!(out.contains("  fr: { 'nav.home': 'Accueil', 'hero.cta': 'Allez', 'hero.title': \"N'attendez pas\", 'hero.sub': 'Sans carte' },\n"), "{out}");
    assert_eq!(parse(Format::Js, path, &out, "fr").unwrap().entries, fr2);
    assert_parses_tsx(&out);
}

/// `glot check` flags hand-rolled i18n that agents add next to the catalogs.
#[test]
fn hand_rolled_i18n_warning() {
    use crate::validate::hand_rolled_i18n;
    let tern = "const A = () => <h2>{language === 'fr' ? 'Nos tarifs' : 'Our pricing'}</h2>;\nconst b = lang!=='en' ? \"Choisir\" : \"Choose\";\n";
    let w = hand_rolled_i18n(tern).unwrap();
    assert_eq!(w.code, IssueCode::HandRolledI18n);
    assert!(w.message.contains("2 language ternaries"), "{}", w.message);
    let dict = "export const translations = {\n  en: { a: 'A' },\n  fr: { a: 'B' },\n};\n";
    assert!(
        hand_rolled_i18n(dict)
            .unwrap()
            .message
            .contains("second translations dictionary")
    );
    // Not i18n: status checks, the pair helper's own definition, a plain object.
    let clean = "if (status === 'ok') go();\nconst tr = (en: string, fr: string) => (language === 'en' ? en : fr);\nconst cfg = { en: true };\n";
    assert_eq!(hand_rolled_i18n(clean), None);
}

/// The patched file must still parse as TSX.
fn assert_parses_tsx(src: &str) {
    let alloc = oxc_allocator::Allocator::default();
    let r = oxc_parser::Parser::new(&alloc, src, oxc_span::SourceType::tsx()).parse();
    assert!(!r.diagnostics.has_errors(), "{src}");
}

/// Field tests on real iOS / gettext projects (anonymized).
#[test]
fn native_field_regressions() {
    let path = Path::new("en.lproj/Localizable.strings");
    // `\U2019` is a .strings escape; Swift's `\u{2019}` (single or doubled backslash) is not: kept verbatim.
    let src = "\"a\" = \"It\\U2019s\";\n\"b\" = \"Next \\\\u{2192}\";\n\"c\" = \"Go \\u{2192}\";\n\"dup\" = \"one\";\n\"dup\" = \"two\";\n";
    let p = parse(Format::Strings, path, src, "en").unwrap();
    assert_eq!(p.entries["a"], "It\u{2019}s");
    assert_eq!(p.entries["b"], "Next \\u{2192}");
    assert_eq!(p.entries["c"], "Go \\u{2192}");
    // A duplicated key reads as its last definition and the file is left untouched.
    assert_eq!(p.entries["dup"], "two");
    let cat = Catalog {
        locale: "en".into(),
        entries: p.entries.clone(),
    };
    assert_eq!(write(Format::Strings, path, Some(src), &cat), src);

    // UTF-16 LE + CRLF (old Xcode): read, patched in the same encoding, appended lines in CRLF.
    let dir = std::env::temp_dir().join(format!("glot-utf16-{}", std::process::id()));
    let file = dir.join("fr.lproj/Localizable.strings");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let crlf = "/* note */\r\n\"a\" = \"C\\U2019est\";\r\n";
    std::fs::write(&file, encode(crlf, Enc::Utf16Le)).unwrap();
    let read_back = read(&file, "fr", Format::Auto).unwrap().unwrap();
    assert_eq!(read_back.entries["a"], "C\u{2019}est");
    let mut cat = Catalog {
        locale: "fr".into(),
        entries: read_back.entries,
    };
    write_path(Format::Strings, &file, &cat).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), encode(crlf, Enc::Utf16Le));
    cat.entries.insert("new".into(), "Nouveau".into());
    write_path(Format::Strings, &file, &cat).unwrap();
    let (text, enc) = decode(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(enc, Enc::Utf16Le);
    assert_eq!(text, format!("{crlf}\"new\" = \"Nouveau\";\r\n"));
    let _ = std::fs::remove_dir_all(&dir);
    // UTF-8 with a BOM.
    assert_eq!(
        decode(b"\xEF\xBB\xBF\"k\" = \"v\";").unwrap().1,
        Enc::Utf8Bom
    );
}

#[test]
fn printf_field_regressions() {
    use crate::validate::{Severity, lint_source};
    let errors = |s: &str, t: &str| -> Vec<IssueCode> {
        validate(s, t, "fr", Syntax::Printf, &Default::default())
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.code)
            .collect()
    };
    use IssueCode::*;
    // Swift escape is text, not an ICU `{2019}` argument.
    assert_eq!(
        errors("Friend\\u{2019}s pass", "Le passeport de l\u{2019}ami"),
        []
    );
    // Python brace fields (gettext python-brace-format) are placeholders; literal braces are text.
    assert_eq!(errors("At least {:d} nodes", "Au moins {:d} nœuds"), []);
    assert_eq!(
        errors("At least {:d} nodes", "Au moins {:s} nœuds"),
        [KeywordTranslated]
    );
    assert_eq!(errors("{} of {}", "{} sur {}"), []);
    assert_eq!(errors("{name!r} missing", "manquant"), [PlaceholderMissing]);
    assert_eq!(
        errors("Use {{x}} or { alone", "Utilisez {{x}} ou { seul"),
        []
    );
    // Literal `<NONE>`-like text in native catalogs is not unbalanced markup.
    assert_eq!(errors("Value <NONE>", "Valeur <NONE>"), []);
    assert_eq!(errors("Value <NONE>", "Valeur"), [TagMismatch]);
    // Native plurals still parsed as ICU.
    assert_eq!(
        errors(
            "{count, plural, one {%d day} other {%d days}}",
            "{count, plural, one {%d jour} other {%d jours}}"
        ),
        []
    );
    // Code left in a catalog by a broken extraction: flagged once on the source.
    assert_eq!(lint_source("— \\(L10n.tr(", "— \\(L10n.tr(").len(), 1);
    assert_eq!(lint_source("Hello", "Hello").len(), 0);
    let w = lint_source("Next \\u{2192}", "Next \\u{2192}");
    assert!(
        w[0].message.contains("\\U2192") && w[0].message.contains('\u{2192}'),
        "{w:?}"
    );
}

#[test]
fn lingui_po() {
    let text = r#"msgid ""
msgstr ""
"Language: fr\n"
"X-Generator: @lingui/cli\n"

#: src/App.tsx:4
msgid "{count, plural, one {# message} other {# messages}}"
msgstr "{count, plural, one {# message} other {# messages}}"

#. js-lingui-explicit-id
#: src/App.tsx:9
msgid "nav.home"
msgstr "Accueil"

#. js-lingui-id: Xk2eEo
#. Greeting on the dashboard
msgid "Hello {name}"
msgstr "Bonjour {name}"
"#;
    let (f, p, cat) = load("src/locales/fr/messages.po", text, "fr");
    assert_eq!(f, Format::Po);
    assert_eq!(p.entries["nav.home"], "Accueil");
    assert_eq!(p.entries["Hello {name}"], "Bonjour {name}");
    // Explicit ids are keys, not source text.
    assert!(!p.implicit.contains_key("nav.home"));
    assert_eq!(p.implicit["Hello {name}"], "Hello {name}");
    // Lingui markers are not translator notes.
    assert_eq!(p.notes["Hello {name}"], "Greeting on the dashboard");
    assert_eq!(p.notes["nav.home"], "References: src/App.tsx:9");
    // ICU messages validate as ICU.
    let k = "{count, plural, one {# message} other {# messages}}";
    let issues = validate(k, &p.entries[k], "fr", Syntax::Icu, &Default::default());
    assert!(
        issues.iter().all(|i| i.code != IssueCode::SyntaxError),
        "{issues:?}"
    );
    assert_eq!(write(f, Path::new("m.po"), Some(text), &cat), text);
}
