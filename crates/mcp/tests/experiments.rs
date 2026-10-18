//! Local MCP experiment tools, in-process: registered, and a clear error without `[server]` (the flows
//! themselves are covered by `xp_flow` unit tests and, against a real server, `crates/server/tests/xp_cli.rs`).
use rmcp::{ServiceExt, model::*};
use serde_json::json;

use glot_mcp::GlotServer;

#[tokio::test]
async fn experiment_tools_need_a_server() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(
        p.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"de\"]\nfiles = [\"{locale}.json\"]\n",
    )
    .unwrap();
    std::fs::write(p.join("en.json"), r#"{"home":{"hero":{"title":"Hi"}}}"#).unwrap();
    std::fs::write(p.join("de.json"), "{}").unwrap();
    let (s, c) = tokio::io::duplex(1 << 16);
    let server = GlotServer::open(p).unwrap();
    tokio::spawn(async move { server.serve(s).await.unwrap().waiting().await });
    let c = ().serve(c).await.unwrap();
    let names: Vec<String> = c
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    for n in [
        "experiment_create",
        "experiment_status",
        "experiment_promote",
    ] {
        assert!(names.contains(&n.to_string()), "{names:?}");
    }
    let r = c
        .call_tool(
            CallToolRequestParams::new("experiment_create")
                .with_arguments(json!({"prefix": "home.hero"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let text = r.content[0].as_text().unwrap().text.clone();
    assert!(
        r.is_error == Some(true) && text.contains("[server]"),
        "{text}"
    );
}
