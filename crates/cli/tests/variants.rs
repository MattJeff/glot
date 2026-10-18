//! Register variants with the mock provider: generated from the base locale, placeholders kept, tracked
//! in glot.lock against the base, regenerated when the base changes, and checked by `glot check`.
use std::fs;

fn glot(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    assert_cmd::Command::cargo_bin("glot")
        .unwrap()
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn variants_follow_their_base() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir(r.join("messages")).unwrap();
    fs::write(
        r.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\nfallback_model = \"mock\"\n[variants]\nfr = [\"informal\"]\n",
    )
    .unwrap();
    fs::write(
        r.join("messages/en.json"),
        r#"{"hello": "Hello {name}, you have {n, plural, one {# file} other {# files}}"}"#,
    )
    .unwrap();

    let out = glot(r, &["translate", "--yes"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fr: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(r.join("messages/fr.json")).unwrap()).unwrap();
    let inf: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(r.join("messages/fr-x-informal.json")).unwrap())
            .unwrap();
    let (fr, inf) = (
        fr["hello"].as_str().unwrap(),
        inf["hello"].as_str().unwrap(),
    );
    // Generated from the French base, not from English.
    assert!(
        inf.contains("⟦fr-x-informal⟧") && inf.contains("⟦fr⟧"),
        "{inf}"
    );
    assert!(inf.contains("{name}") && inf.contains("many {"), "{inf}");
    let lock = fs::read_to_string(r.join("glot.lock")).unwrap();
    let lock: serde_json::Value = serde_json::from_str(&lock).unwrap();
    assert_eq!(
        lock["entries"]["fr-x-informal"]["hello"]["source_hash"],
        glot_core::hash::hash(fr)
    );

    // Up to date: nothing to do.
    let out = glot(r, &["translate", "--yes", "--dry-run"]);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("fr-x-informal"));

    // A broken variant (placeholder renamed) fails `glot check`.
    fs::write(
        r.join("messages/fr-x-informal.json"),
        r#"{"hello": "Salut {nom}, tu as {n, plural, one {# fichier} many {# fichiers} other {# fichiers}}"}"#,
    )
    .unwrap();
    let out = glot(r, &["check"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("fr-x-informal"));
}
