//! `glot labs store` end to end with the mock provider on a tiny iOS app: init from Info.plist +
//! Localizable.strings, generate per `.lproj` locale (limits enforced, overlong fields retried),
//! check, release notes from CHANGELOG.md.
use std::fs;
use std::path::Path;
use std::process::Command;

fn glot(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_glot"))
        .args(args)
        .args(["--provider", "mock"])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let log =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), log)
}

#[test]
fn store_listing_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    fs::create_dir_all(r.join("App/en.lproj")).unwrap();
    fs::create_dir_all(r.join("App/fr.lproj")).unwrap();
    fs::create_dir_all(r.join("App/zh-Hans.lproj")).unwrap();
    fs::write(r.join("App/Info.plist"), "<?xml version=\"1.0\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleDisplayName</key><string>Trip Visa</string><key>LSApplicationCategoryType</key><string>public.app-category.travel</string></dict></plist>").unwrap();
    fs::write(
        r.join("App/en.lproj/Localizable.strings"),
        "\"a\" = \"Check visa requirements\";\n\"b\" = \"Compare passports side by side\";\n\"c\" = \"Embassy contacts and border rules\";\n",
    )
    .unwrap();

    let (ok, log) = glot(r, &["labs", "store", "init"]);
    assert!(ok, "{log}");
    let md = r.join("fastlane/metadata");
    assert_eq!(
        fs::read_to_string(md.join("en-US/name.txt")).unwrap(),
        "Trip Visa"
    );
    // A subtitle at 29/30: `⟦fr-FR⟧ …` overflows, the retry must shorten it.
    fs::write(
        md.join("en-US/subtitle.txt"),
        "Embassy contacts & entry tips",
    )
    .unwrap();
    fs::write(
        md.join("en-US/keywords.txt"),
        "passport,schengen,evisa,consulate",
    )
    .unwrap();

    let (ok, log) = glot(r, &["labs", "store", "generate", "--competitors", "Sherpa"]);
    assert!(ok, "{log}");
    assert!(log.contains("fr-FR") && log.contains("zh-Hans"), "{log}");
    for l in ["fr-FR", "zh-Hans"] {
        let sub = fs::read_to_string(md.join(l).join("subtitle.txt")).unwrap();
        assert!(sub.chars().count() <= 30, "{l}: {sub}");
        let kw = fs::read_to_string(md.join(l).join("keywords.txt")).unwrap();
        assert!(
            kw.len() <= 100
                && !kw.contains(' ')
                && !kw
                    .split(',')
                    .any(|t| ["visa", "trip", "embassy"].contains(&t)),
            "{l}: {kw}"
        );
    }
    let (ok, log) = glot(r, &["labs", "store", "check"]);
    assert!(ok, "{log}");
    assert!(log.contains("keywords"), "{log}");

    // A hand edit that breaks the rules is caught.
    fs::write(md.join("fr-FR/keywords.txt"), "trip, sherpa").unwrap();
    let (ok, log) = glot(r, &["labs", "store", "check", "--competitors", "Sherpa"]);
    assert!(!ok, "{log}");
    assert!(
        log.contains("spaces around commas") && log.contains("competitor brand"),
        "{log}"
    );

    fs::write(
        r.join("CHANGELOG.md"),
        "## 1.1.0\n- Dark mode\n- chore: lint\n",
    )
    .unwrap();
    let (ok, log) = glot(r, &["labs", "store", "notes", "--locales", "fr"]);
    assert!(ok, "{log}");
    let notes = fs::read_to_string(md.join("fr-FR/release_notes.txt")).unwrap();
    assert_eq!(notes, "• ⟦fr-FR⟧ Dark mode");
}
