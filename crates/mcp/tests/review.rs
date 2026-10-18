//! MCP review_translation / mine_glossary with the mock provider.
use rmcp::{ServiceExt, model::*};
use serde_json::{Value, json};

#[tokio::test]
async fn review_and_mine_tools() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("glot.toml"), "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n").unwrap();
    std::fs::create_dir_all(p.join("messages")).unwrap();
    std::fs::write(
        p.join("messages/en.json"),
        r#"{"a":"Open the Workspace","b":"Delete the Workspace"}"#,
    )
    .unwrap();
    std::fs::write(
        p.join("messages/fr.json"),
        r#"{"a":"⟦fr⟧ Open the Workspace","b":"⟦fr⟧ Supprimer"}"#,
    )
    .unwrap();

    let (s, c) = tokio::io::duplex(1 << 16);
    let server = glot_mcp::GlotServer::open(p).unwrap();
    tokio::spawn(async move { server.serve(s).await.unwrap().waiting().await });
    let c = ().serve(c).await.unwrap();
    let call = async |name: &'static str, args: Value| -> Value {
        let r = c
            .call_tool(
                CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert!(!r.is_error.unwrap_or(false), "{name} failed: {r:?}");
        serde_json::from_str(&r.content[0].as_text().unwrap().text).unwrap()
    };

    let v = call("review_translation", json!({"locales": ["fr"]})).await;
    assert_eq!(
        (v["total"].as_u64(), v["same"].as_u64()),
        (Some(2), Some(1))
    );
    assert_eq!(v["results"][0]["key"], "b"); // non-`same` first
    assert_eq!(v["results"][0]["verdict"], "different");

    let v = call("mine_glossary", json!({})).await;
    let w = v["terms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["term"]["term"] == "Workspace")
        .unwrap();
    assert_eq!(w["inconsistent"]["fr"], json!(["b"]));
    call("mine_glossary", json!({"apply": true})).await;
    let fr = std::fs::read_to_string(p.join("messages/fr.json")).unwrap();
    assert!(fr.contains("⟦fr⟧ Delete the Workspace"), "{fr}");
    assert!(
        std::fs::read_to_string(p.join("glossary.toml"))
            .unwrap()
            .contains("Workspace")
    );
}
