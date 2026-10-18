//! `glot coverage --render` end to end: a static app page, a Polish plural whose `few` branch is too long
//! for its measured 100px box. Needs node + packages/visual-qa deps.
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let body = r#"<!doctype html><html lang="en"><body style="font:16px Arial"><h1>App</h1></body></html>"#;
            let _ = write!(
                &stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

fn coverage(dir: &std::path::Path, args: &[&str]) -> (bool, serde_json::Value) {
    let out = assert_cmd::Command::cargo_bin("glot")
        .unwrap()
        .current_dir(dir)
        .arg("coverage")
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.code().is_some_and(|c| c <= 1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        out.status.success(),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

fn visual(v: &serde_json::Value, locale: &str, id: &str) -> serde_json::Value {
    v["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["locale"] == locale && b["id"] == id)
        .unwrap_or_else(|| panic!("no branch {locale} {id}"))["visual"]
        .clone()
}

#[test]
fn render_flags_the_polish_few_branch() {
    let vqa = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/visual-qa/node_modules/playwright");
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
        || !vqa.exists()
    {
        eprintln!("skipped: needs node and `pnpm install` in packages/visual-qa");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir_all(r.join("messages")).unwrap();
    fs::create_dir_all(r.join(".glot")).unwrap();
    fs::write(
        r.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"pl\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
    )
    .unwrap();
    fs::write(
        r.join("messages/en.json"),
        r#"{"files": "{n, plural, one {# file} other {# files}}", "title": "Files"}"#,
    )
    .unwrap();
    let pl = r#"{"files": "{n, plural, one {# plik} few {# pliki do pobrania teraz} many {# plików} other {# pliku}}", "title": "Pliki"}"#;
    fs::write(r.join("messages/pl.json"), pl).unwrap();
    fs::write(
        r.join(".glot/layout.json"),
        r#"{"files": {"max_width_px": 100, "max_lines": 1, "font": {"family": "Arial", "size_px": 16, "weight": 400, "letter_spacing_px": 0}}}"#,
    )
    .unwrap();

    // Before any render: every branch is unseen.
    let (_, v) = coverage(r, &[]);
    assert_eq!(v["summary"]["pl"]["few"]["branches"], 1);
    assert_eq!(v["summary"]["pl"]["few"]["visual"], 0);
    assert!(visual(&v, "pl", "n=few").is_null());

    let port = serve();
    let base = format!("http://127.0.0.1:{port}");
    let (ok, v) = coverage(r, &["--render", "--base-url", &base, "--routes", "/"]);
    assert!(!ok, "an overflowing branch fails the run");
    assert_eq!(visual(&v, "pl", "n=few"), "overflow");
    assert_eq!(visual(&v, "pl", "n=one"), "ok");
    assert_eq!(visual(&v, "en", "n=other"), "ok");
    assert_eq!(v["summary"]["pl"]["few"]["failing"], 1);

    // Results persist in .glot/coverage.json; editing the branch text makes it unseen again.
    fs::write(
        r.join("messages/pl.json"),
        pl.replace("# pliki do pobrania teraz", "# pliki"),
    )
    .unwrap();
    let (ok, v) = coverage(r, &[]);
    assert!(ok);
    assert!(visual(&v, "pl", "n=few").is_null());
    assert_eq!(visual(&v, "pl", "n=many"), "ok");
}
