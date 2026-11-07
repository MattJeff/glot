//! Mobile / desktop formats: Qt Linguist `.ts`, moko-resources `<plural>`, Qt / slang
//! placeholders, moko / slang path layouts.
use std::path::Path;

use super::*;
use crate::validate::{IssueCode, Severity, Syntax, validate};

const QT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/formats/app_fr.ts"
));
const K: &str = "MainWindow\u{4}";

fn qt(text: &str) -> Parsed {
    let f = native(Format::Qt, Path::new("app_fr.ts")).unwrap();
    parse(f, Path::new("app_fr.ts"), text, "fr").unwrap()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("glot-mobile-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cat(locale: &str, p: &Parsed) -> Catalog {
    Catalog {
        locale: locale.into(),
        entries: p.entries.clone(),
    }
}

#[test]
fn qt_read_roundtrip_edit() {
    let p = qt(QT);
    assert_eq!(p.entries[&format!("{K}Hello %1")], "Bonjour %1");
    assert_eq!(p.entries[&format!("{K}&File")], "");
    assert_eq!(
        p.entries[&format!("{K}%n file(s)")],
        "{count, plural, one {%n fichier} other {%n fichiers}}"
    );
    // `<comment>` disambiguates (part of the key) and is a note.
    assert_eq!(p.entries[&format!("{K}Open\u{4}verb")], "Ouvrir");
    assert!(p.notes[&format!("{K}Open\u{4}verb")].contains("verb"));
    assert!(p.notes[&format!("{K}Hello %1")].contains("../mainwindow.cpp:12"));
    assert_eq!(p.implicit[&format!("{K}Hello %1")], "Hello %1");

    let mut c = cat("fr", &p);
    assert_eq!(qt::write(Some(QT), &c), QT, "byte for byte");

    // Filling an unfinished entry drops `type="unfinished"`; a new key lands in its context.
    c.entries.insert(format!("{K}&File"), "&Fichier <x>".into());
    c.entries.insert(
        format!("{K}%n file(s)"),
        "{count, plural, one {%n doc} other {%n docs}}".into(),
    );
    c.entries.insert(format!("{K}Save"), "Enregistrer".into());
    c.entries
        .insert("About\u{4}Version %1".into(), "Version %1".into());
    let out = qt::write(Some(QT), &c);
    assert!(out.contains("<translation>&amp;Fichier &lt;x&gt;</translation>"));
    assert!(!out.contains("unfinished"));
    assert!(out.contains("<numerusform>%n doc</numerusform>\n            <numerusform>%n docs</numerusform>\n        </translation>"));
    assert!(out.contains("<source>Save</source>"));
    assert!(out.contains("<name>About</name>"));
    let back = qt(&out);
    assert_eq!(back.entries, c.entries);
    assert_eq!(qt::write(Some(&out), &c), out);

    // A removed key drops its `<message>`.
    c.entries.remove(&format!("{K}Save"));
    assert!(!qt::write(Some(&out), &c).contains("Save"));
}

#[test]
fn qt_new_file_and_unfinished() {
    let mut c = Catalog::new("de");
    c.entries.insert(format!("{K}Hello %1"), "Hallo %1".into());
    c.entries.insert(
        format!("{K}%n file(s)"),
        "{count, plural, one {%n Datei} other {%n Dateien}}".into(),
    );
    let out = qt::write(None, &c);
    assert!(out.starts_with("<?xml") && out.contains("language=\"de\""));
    assert_eq!(qt(&out).entries, c.entries);
    // `Format::Auto` resolves an existing XML `.ts` to Qt, TypeScript stays JS.
    let dir = tmp("qt");
    let (a, b) = (dir.join("app_de.ts"), dir.join("i18n.ts"));
    std::fs::write(&a, &out).unwrap();
    std::fs::write(&b, "export default {};").unwrap();
    assert_eq!(native(Format::Auto, &a), Some(Format::Qt));
    assert_eq!(native(Format::Auto, &b), Some(Format::Js));

    let stale = QT.replace(
        "<translation>Ouvrir</translation>",
        "<translation type=\"unfinished\">Ouvrir</translation>",
    );
    assert!(qt(&stale).stale.contains(&format!("{K}Open\u{4}verb")));
}

#[test]
fn moko_plural_tag() {
    let src = "<resources>\n    <plural name=\"apples\">\n        <item quantity=\"one\">%d apple</item>\n        <item quantity=\"other\">%d apples</item>\n    </plural>\n</resources>\n";
    let p = android::parse(src).unwrap();
    assert_eq!(
        p.entries["apples"],
        "{count, plural, one {%d apple} other {%d apples}}"
    );
    let mut c = cat("en", &p);
    assert_eq!(android::write(Some(src), &c), src);
    c.entries.insert(
        "apples".into(),
        "{count, plural, one {%d pomme} other {%d pommes}}".into(),
    );
    let out = android::write(Some(src), &c);
    assert!(out.contains("</plural>\n") && !out.contains("</plurals>"));
    assert_eq!(android::parse(&out).unwrap().entries, c.entries);
}

#[test]
fn qt_and_slang_placeholders() {
    let errs = |s: &str, t: &str| -> Vec<IssueCode> {
        validate(s, t, "fr", Syntax::Printf, &Default::default())
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.code)
            .collect()
    };
    use IssueCode::*;
    assert_eq!(errs("Hello %1, %2 left", "%2 restants, salut %1"), []);
    assert_eq!(
        errs("Hello %1", "Bonjour %2"),
        [PlaceholderMissing, PlaceholderExtra]
    );
    assert_eq!(errs("Total %L1", "Total"), [PlaceholderMissing]);
    assert_eq!(errs("%n file(s)", "fichier(s)"), [PlaceholderMissing]);
    // Plain percent text stays text.
    assert_eq!(errs("50% off, 100%", "50 % de remise"), []);
    // slang `$name` / `${name}`; `%1$s` is still printf.
    assert_eq!(errs("Hi $name, ${count}!", "Salut $name, ${count} !"), []);
    assert_eq!(
        errs("Hi $name", "Salut $nom"),
        [PlaceholderMissing, PlaceholderExtra]
    );
    assert_eq!(errs("%1$s and %2$s", "%2$s et %1$s"), []);
    assert_eq!(errs("Costs US$5", "Coûte 5 $"), []);
}

#[test]
fn moko_and_slang_paths() {
    let dir = tmp("paths");
    let r = dir.as_path();
    std::fs::write(r.join("strings.i18n.json"), "{}").unwrap();
    let cfg: crate::config::Config = toml::from_str(
        "source_locale = \"en\"\ntarget_locales = [\"pt-BR\"]\nfiles = [\"shared/src/commonMain/moko-resources/{locale}/strings.xml\", \"strings_{locale}.i18n.json\"]\n",
    )
    .unwrap();
    let rel = |l: &str| -> Vec<String> {
        cfg.catalog_paths(r, l)
            .iter()
            .map(|p| p.strip_prefix(r).unwrap().to_string_lossy().into_owned())
            .collect()
    };
    assert_eq!(
        rel("en"),
        [
            "shared/src/commonMain/moko-resources/base/strings.xml",
            "strings.i18n.json"
        ]
    );
    assert_eq!(
        rel("pt-BR"),
        [
            "shared/src/commonMain/moko-resources/pt-rBR/strings.xml",
            "strings_pt-BR.i18n.json"
        ]
    );
}
