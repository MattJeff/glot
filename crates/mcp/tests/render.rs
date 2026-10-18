//! MCP render_check: a bidi error with its `fixed` value, accepted by set_translation.
use rmcp::{ServiceExt, model::*};
use serde_json::{Value, json};

#[tokio::test]
async fn render_check_returns_applicable_fixes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("glot.toml"), "source_locale = \"en\"\ntarget_locales = [\"ar\"]\nfiles = [\"messages/{locale}.json\"]\nsyntax = \"printf\"\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n").unwrap();
    std::fs::create_dir_all(p.join("messages")).unwrap();
    std::fs::write(p.join("messages/en.json"), r#"{"a":"%@ has %d visas"}"#).unwrap();
    std::fs::write(p.join("messages/ar.json"), r#"{"a":"%@ %d تأشيرة"}"#).unwrap();

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

    let v = call("render_check", json!({})).await;
    assert_eq!(v["errors"].as_u64(), Some(2), "{v}"); // spillover + first-strong (native)
    let fixed = v["findings"][0]["fixed"].as_str().unwrap().to_string();
    assert_eq!(fixed, "\u{2068}%@\u{2069} %d تأشيرة");
    call(
        "set_translation",
        json!({"locale": "ar", "key": "a", "value": fixed}),
    )
    .await;
    let v = call("render_check", json!({})).await;
    assert_eq!(v["errors"].as_u64(), Some(0), "{v}");
}
