use rmcp::{RoleClient, ServiceExt, model::*, service::RunningService};
use serde_json::{Value, json};

use glot_mcp::GlotServer;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(
        p.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\nbatch_size = 40\n",
    )
    .unwrap();
    std::fs::create_dir_all(p.join("messages")).unwrap();
    std::fs::write(
        p.join("messages/en.json"),
        r#"{"common":{"save":"Save","hello":"Hello {name}"}}"#,
    )
    .unwrap();
    std::fs::write(
        p.join("messages/fr.json"),
        r#"{"common":{"save":"Enregistrer"}}"#,
    )
    .unwrap();
    dir
}

async fn client(dir: &std::path::Path) -> RunningService<RoleClient, ()> {
    let (s, c) = tokio::io::duplex(1 << 16);
    let server = GlotServer::open(dir).unwrap();
    tokio::spawn(async move { server.serve(s).await.unwrap().waiting().await });
    ().serve(c).await.unwrap()
}

async fn call(
    c: &RunningService<RoleClient, ()>,
    name: &'static str,
    args: Value,
) -> (bool, Value) {
    let r = c
        .call_tool(
            CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let text = r
        .content
        .iter()
        .find_map(|b| b.as_text().map(|t| t.text.clone()))
        .unwrap_or_default();
    (
        r.is_error.unwrap_or(false),
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn end_to_end() {
    let dir = project();
    let c = client(dir.path()).await;

    let tools: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for t in [
        "status",
        "get_context",
        "propose_string",
        "add_string",
        "translate",
        "check",
        "set_translation",
        "find_hardcoded",
        "glossary_lookup",
        "glossary_upsert",
        "remove_keys",
    ] {
        assert!(tools.contains(&t.to_string()), "missing tool {t}");
    }

    let (err, v) = call(
        &c,
        "propose_string",
        json!({"text": "Save", "suggested_key": "settings.save"}),
    )
    .await;
    assert!(!err, "{v}");
    assert_eq!(v["duplicates"][0]["key"], "common.save", "{v}");
    assert!(v["advice"][0].as_str().unwrap().contains("common.save"));

    let (err, v) = call(
        &c,
        "add_string",
        json!({"key": "settings.title", "text": "Settings", "note": "page heading"}),
    )
    .await;
    assert!(!err, "{v}");
    assert_eq!(v["action"], "created");

    let (err, v) = call(
        &c,
        "add_string",
        json!({"key": "settings.title", "text": "Preferences"}),
    )
    .await;
    assert!(err, "overwrite must be refused: {v}");

    let (err, v) = call(
        &c,
        "add_string",
        json!({"key": "bad", "text": "Hello {name"}),
    )
    .await;
    assert!(err, "invalid syntax must be refused: {v}");

    let (err, v) = call(&c, "translate", json!({})).await;
    assert!(!err, "{v}");
    assert!(
        v.get("written").is_some() && v.get("rejected_total").is_some(),
        "{v}"
    );
    assert_eq!(v["written"]["fr"], 2, "{v}");

    let (err, v) = call(&c, "check", json!({"locale": "fr"})).await;
    assert!(!err, "{v}");
    assert!(v["total"].is_number());

    let (err, _) = call(&c, "check", json!({"locale": "xx"})).await;
    assert!(err);

    let r = c
        .read_resource(ReadResourceRequestParams::new("glot://catalog/fr"))
        .await
        .unwrap();
    assert!(!r.contents.is_empty());

    c.cancel().await.unwrap();
}

/// Plugins and MCP registries start the server in repos without glot.toml: it must come up.
#[test]
fn starts_without_a_project() {
    let dir = tempfile::tempdir().unwrap();
    assert!(glot_mcp::GlotServer::open(dir.path()).is_ok());
}
