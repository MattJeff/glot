//! `glot labs distill` end to end on the mock provider (local lane and cloud lane both mocked).
use assert_cmd::Command;
use std::fs;

#[test]
fn distill_export_eval_translate() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    fs::write(
        r.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"cloud-mock\"\n",
    )
    .unwrap();
    fs::create_dir_all(r.join("messages")).unwrap();
    fs::write(
        r.join("messages/en.json"),
        r#"{"a":"Save changes","b":"Delete project","c":"Invite your team members today","d":"Billing settings","e":"Open the billing settings page now"}"#,
    )
    .unwrap();
    let glot = || {
        let mut c = Command::cargo_bin("glot").unwrap();
        c.current_dir(r).env("NO_COLOR", "1").env_remove("RUST_LOG");
        c
    };
    for (k, v) in [
        ("a", "Enregistrer les modifications"),
        ("b", "Supprimer le projet"),
        ("c", "Invitez les membres de votre équipe"),
        ("d", "Paramètres de facturation"),
    ] {
        glot().args(["set", "fr", k, v]).assert().success();
    }

    let out = glot().args(["labs", "distill", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["train"].as_u64().unwrap() + v["eval"].as_u64().unwrap(),
        4
    );
    assert!(r.join(".glot/distill/train.jsonl").exists());

    let out = glot()
        .args([
            "labs",
            "distill",
            "eval",
            "--provider",
            "mock",
            "--model",
            "local-mock",
            "--cloud-provider",
            "mock",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("local+rag") && text.contains("local-mock + RAG"),
        "{text}"
    );
    let toml = fs::read_to_string(r.join("glot.toml")).unwrap();
    assert!(toml.contains("# >>> glot labs distill"), "{toml}");

    glot()
        .args([
            "labs",
            "distill",
            "translate",
            "--provider",
            "mock",
            "--model",
            "local-mock",
        ])
        .assert()
        .success();
    let fr = fs::read_to_string(r.join("messages/fr.json")).unwrap();
    assert!(fr.contains("Open the billing settings page now"), "{fr}");
    glot().arg("check").assert().success();
}
