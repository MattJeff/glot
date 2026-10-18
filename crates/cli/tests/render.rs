//! `glot labs render`: glyph gaps of a repo webfont, bidi errors (exit 1), `--fix` through the
//! validator (FSI/PDI, no-break spaces), then clean and `glot check` still green.
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
fn render_finds_fixes_and_stays_valid() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join("messages")).unwrap();
    fs::create_dir_all(r.join("public/fonts")).unwrap();
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../core/tests/fixtures/Geist-Regular.ttf"
        ),
        r.join("public/fonts/Geist-Regular.ttf"),
    )
    .unwrap();
    fs::write(
        r.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"ar\", \"fr\", \"ja\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
    )
    .unwrap();
    fs::write(
        r.join("messages/en.json"),
        r#"{"a": "{name} has {count, number} visas", "b": "Saved!", "c": "Send"}"#,
    )
    .unwrap();
    fs::write(
        r.join("messages/ar.json"),
        r#"{"a": "{name} {count, number} تأشيرة", "b": "تم الحفظ!", "c": "إرسال"}"#,
    )
    .unwrap();
    fs::write(
        r.join("messages/fr.json"),
        r#"{"a": "{name} a {count, number} visas", "b": "Enregistré !", "c": "Envoyer"}"#,
    )
    .unwrap();
    fs::write(
        r.join("messages/ja.json"),
        r#"{"a": "{name}は{count, number}件のビザ", "b": "保存しました", "c": "送信"}"#,
    )
    .unwrap();

    let out = glot(r, &["labs", "render", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["fonts"][0]["family"], "Geist");
    let gap = |l: &str| {
        v["glyphs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["locale"] == l)
            .cloned()
    };
    assert_eq!(gap("ja").unwrap()["keys"], 3);
    assert!(
        gap("ar").unwrap()["fallback"]
            .as_str()
            .unwrap()
            .contains("Arabic")
    );
    assert!(gap("en").is_none() && gap("fr").is_none());
    let checks: Vec<(String, String)> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                format!(
                    "{}:{}",
                    f["locale"].as_str().unwrap(),
                    f["key"].as_str().unwrap()
                ),
                f["check"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert!(
        checks.contains(&("ar:a".into(), "bidi".into())),
        "{checks:?}"
    );
    assert!(
        checks.contains(&("fr:b".into(), "nbsp".into())),
        "{checks:?}"
    );

    let out = glot(r, &["labs", "render", "--fix"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let ar = fs::read_to_string(r.join("messages/ar.json")).unwrap();
    assert!(ar.contains("\u{2068}{name}\u{2069}"), "{ar}");
    let fr = fs::read_to_string(r.join("messages/fr.json")).unwrap();
    assert!(fr.contains("Enregistré\u{202F}!"), "{fr}");
    assert!(glot(r, &["check"]).status.success());
    // Fonts gaps remain (warnings), nothing fixable left.
    let v: serde_json::Value =
        serde_json::from_slice(&glot(r, &["labs", "render", "--json"]).stdout).unwrap();
    assert_eq!(
        (v["errors"].as_u64(), v["fixable"].as_u64()),
        (Some(0), Some(0)),
        "{v}"
    );
}
