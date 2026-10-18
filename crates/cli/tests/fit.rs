//! `glot fit` end to end: a page rendered from the catalogs on each request, a fixed-width button
//! that overflows in German, the mock provider shortening it. Needs node + packages/visual-qa deps.
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;

fn serve(root: PathBuf) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split(' ').nth(1).unwrap_or("/");
            let locale = if path.starts_with("/de") { "de" } else { "en" };
            let cat: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(root.join(format!("messages/{locale}.json"))).unwrap(),
            )
            .unwrap();
            let body = format!(
                r#"<!doctype html><html lang="{locale}"><body style="font:16px sans-serif">
<section><h1>{}</h1><button style="width:120px;white-space:nowrap;overflow:hidden">{}</button></section></body></html>"#,
                cat["home"]["title"].as_str().unwrap(),
                cat["home"]["cta"].as_str().unwrap()
            );
            let mut s = &stream;
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

#[test]
fn fit_shortens_overflowing_button_until_it_fits() {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skipped: node not installed");
        return;
    }
    let vqa = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/visual-qa/node_modules/playwright");
    if !vqa.exists() {
        eprintln!("skipped: run `pnpm install` in packages/visual-qa to enable this test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::create_dir(r.join("messages")).unwrap();
    fs::write(
        r.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"de\"]\nfiles = [\"messages/{locale}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
    )
    .unwrap();
    fs::write(
        r.join("messages/en.json"),
        r#"{"home": {"title": "Welcome", "cta": "Start free trial"}}"#,
    )
    .unwrap();
    fs::write(
        r.join("messages/de.json"),
        r#"{"home": {"title": "Willkommen", "cta": "Kostenlose Testversion jetzt sofort starten"}}"#,
    )
    .unwrap();
    let port = serve(r.to_path_buf());

    let out = assert_cmd::Command::cargo_bin("glot")
        .unwrap()
        .current_dir(r)
        .args([
            "fit",
            "--base-url",
            &format!("http://127.0.0.1:{port}"),
            "--routes",
            "/",
            "--yes",
            "--reload-ms",
            "0",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let de: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(r.join("messages/de.json")).unwrap()).unwrap();
    let cta = de["home"]["cta"].as_str().unwrap();
    assert!(cta.len() < 20 && cta.starts_with("Kostenlose"), "{cta}");
    assert_eq!(de["home"]["title"], "Willkommen");

    let report = fs::read_to_string(r.join(".glot/fit/report.md")).unwrap();
    // Converged: at most 2 write iterations, before/after crops side by side.
    assert!(
        report.contains("`home.cta`") && report.contains("✓ fits after 1 iteration"),
        "{report}"
    );
    assert!(!r.join(".glot/fit/run-3").exists() || report.contains("✓ fits after 2"));
    let imgs: Vec<&str> = report
        .match_indices("](run-")
        .map(|(i, _)| &report[i + 2..i + 40])
        .collect();
    assert!(
        imgs.len() == 2 && imgs[0].starts_with("run-1/") && imgs[1].starts_with("run-2/"),
        "{report}"
    );
}
