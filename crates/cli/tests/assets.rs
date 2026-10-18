//! `glot assets` end to end on examples/assets-demo with the mock provider.
use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn glot(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("glot").unwrap();
    c.current_dir(dir)
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG");
    c
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy(&src, &dst);
        } else if src.extension().is_none_or(|x| x != "sh") {
            fs::copy(&src, &dst).unwrap();
        }
    }
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().display().to_string(),
                    fs::read(&p).unwrap(),
                ));
            }
        }
    }
    let mut out = vec![];
    walk(root, root, &mut out);
    out.sort();
    out
}

#[test]
fn assets_pipeline_renders_every_locale_and_is_idempotent() {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/assets-demo");
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    copy(&demo, r);

    let out = glot(r).args(["assets", "--yes"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The tight SVG badge is rejected as too wide and retried before being written.
    assert!(stdout.contains("retried to fit"), "{stdout}");
    assert!(stdout.contains("<Html> has no `dir` prop"), "{stdout}");

    let welcome = fs::read_to_string(r.join("emails/welcome.tsx")).unwrap();
    assert!(welcome.contains(
        r#"export const subject = (t: (key: string) => string) => t("emails.welcome.subject");"#
    ));
    assert!(welcome.contains(r#"useTranslations("emails.welcome")"#));
    let en = fs::read_to_string(r.join("messages/en.json")).unwrap();
    for k in [
        "\"subject\": \"Welcome to Acme, your workspace is ready\"",
        "\"badge\": \"Free plan\"",
        "\"launch\"",
    ] {
        assert!(en.contains(k), "{k} not in {en}");
    }
    let layout = fs::read_to_string(r.join(".glot/layout.json")).unwrap();
    assert!(layout.contains("svg.onboarding.badge") && layout.contains("emails.welcome.subject"));

    for loc in ["en", "fr", "de", "ja", "ar"] {
        for f in [
            format!("public/og/{loc}/launch.png"),
            format!("public/img/{loc}/onboarding.png"),
            format!("public/img/{loc}/onboarding.svg"),
            format!(".glot/emails/welcome/{loc}.html"),
            format!(".glot/emails/invoice_paid/{loc}.html"),
        ] {
            assert!(fs::metadata(r.join(&f)).is_ok_and(|m| m.len() > 0), "{f}");
        }
    }
    let ar = fs::read_to_string(r.join(".glot/emails/welcome/ar.html")).unwrap();
    assert!(ar.contains(r#"<html lang="ar" dir="rtl">"#));
    assert!(ar.contains(r#"href="https://acme.example/docs""#));
    assert!(
        fs::read_to_string(r.join("public/img/de/onboarding.svg"))
            .unwrap()
            .contains("⟦de⟧")
    );

    let before = snapshot(r);
    glot(r).args(["assets", "--yes"]).assert().success();
    assert!(before == snapshot(r), "second run changed files");
}

#[test]
fn assets_check_fails_on_a_too_long_subject() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::write(r.join("glot.toml"), "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n").unwrap();
    fs::create_dir_all(r.join("messages")).unwrap();
    fs::create_dir_all(r.join("emails")).unwrap();
    fs::write(r.join("messages/en.json"), "{}").unwrap();
    fs::write(
        r.join("emails/reset.html"),
        "<html><head><title>Reset</title></head><body><p>Click below to choose a new password.</p><a href=\"{{url}}\">Reset password</a></body></html>",
    )
    .unwrap();
    glot(r).args(["assets", "--yes"]).assert().success();
    // A hand-edited catalog blows the subject budget: caught by `glot assets check`.
    let fr = r.join("messages/fr.json");
    let text = fs::read_to_string(&fr).unwrap().replace(
        "⟦fr⟧ Reset\"",
        "⟦fr⟧ Réinitialisez votre mot de passe maintenant avant que le lien expire\"",
    );
    fs::write(&fr, text).unwrap();
    let out = glot(r).args(["assets", "check"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("subject is 73 characters"), "{stdout}");
    assert!(stdout.contains("too_wide"), "{stdout}");
}
