//! `glot labs panel` end to end on a hand-made capture (no browser), mock provider.
use assert_cmd::Command;
use serde_json::{Value, json};
use std::fs;

#[test]
fn panel_scores_debt_and_ties_problems_to_keys() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    fs::write(
        root.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("messages")).unwrap();
    fs::write(
        root.join("messages/en.json"),
        r#"{"hero":{"title":"Get paid faster","cta":"Start free trial"},"price":"From {amount} per month"}"#,
    )
    .unwrap();
    fs::write(
        root.join("messages/fr.json"),
        r#"{"hero":{"title":"⟦fr⟧ Get paid faster","cta":"⟦fr⟧ Start free trial"},"price":"À partir de {amount} par mois"}"#,
    )
    .unwrap();
    let page = |l: &str, title: &str, cta: &str, price: &str| {
        json!({"route": "/", "locale": l, "url": format!("http://x/{l}"), "screenshot": "screenshots/none.png",
               "title": "Acme", "headline": title, "text": format!("{title}\n{price}\n{cta}"),
               "ctas": [{"id": 0, "text": cta, "href": "/signup", "key": "hero.cta"}]})
    };
    fs::create_dir_all(root.join(".glot/panel")).unwrap();
    let cap = json!({"config": {"routes": [{"route": "/", "value_proposition": "Invoicing that gets you paid faster"}]},
        "pages": [page("en", "Get paid faster", "Start free trial", "From $29 per month"),
                  page("fr", "⟦fr⟧ Get paid faster", "⟦fr⟧ Start free trial", "À partir de $29 par mois")]});
    fs::write(root.join(".glot/panel/capture.json"), cap.to_string()).unwrap();

    let out = Command::cargo_bin("glot")
        .unwrap()
        .current_dir(root)
        .env("NO_COLOR", "1")
        .args(["labs", "panel", "--reuse-capture", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("estimated ≤ $0.0000"));
    let r: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(r["locales"]["en"]["comprehension"], 100.0);
    assert_eq!(r["locales"]["fr"]["debt"], 75.0); // 5-second 0.5, CTA missed: (0.5 + 0) / 2
    let p = r["problems"].as_array().unwrap();
    let title = p.iter().find(|p| p["key"] == "hero.title").unwrap();
    assert_eq!(
        (title["kind"].as_str(), title["valid"].as_bool()),
        (Some("non_native"), Some(true))
    );
    assert_eq!(title["suggestion"], "Get paid faster");
    assert!(
        p.iter()
            .any(|p| p["kind"] == "cta" && p["key"] == "hero.cta")
    );
    let cur = p.iter().find(|p| p["kind"] == "cultural").unwrap();
    assert_eq!(
        (cur["quote"].as_str(), cur["key"].as_str()),
        (Some("$29"), Some("price"))
    );
    let html = fs::read_to_string(root.join(".glot/panel/report.html")).unwrap();
    assert!(html.contains("75 (high)") && html.contains("hero.title"));

    // Budget below the estimate: refused before any call.
    let toml = fs::read_to_string(root.join("glot.toml")).unwrap();
    fs::write(
        root.join("glot.toml"),
        toml.replace(
            "\"mock\"\nmodel = \"mock\"",
            "\"anthropic\"\nmodel = \"claude-sonnet-5-5\"",
        ),
    )
    .unwrap();
    let out = Command::cargo_bin("glot")
        .unwrap()
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("ANTHROPIC_API_KEY", "unused")
        .args([
            "labs",
            "panel",
            "--reuse-capture",
            "--live",
            "--budget",
            "0.001",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("exceeds the budget"));
}
