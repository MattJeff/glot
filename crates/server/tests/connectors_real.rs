//! `connectors doctor` against a REAL platform, from a terminal: the same read-only diagnosis as the
//! dashboard's "Test connection" (auth, languages, first page of content, translations read back,
//! platform-specific checks), printed line by line. Nothing is written to the platform and no secret is
//! printed. Ignored by default: it needs credentials.
//!
//! ```sh
//! GLOT_REAL_KIND=wordpress \
//! GLOT_REAL_SECRETS='{"url":"https://example.com","username":"editor","app_password":"xxxx xxxx"}' \
//! cargo test -p glot-server --test connectors_real -- --ignored --nocapture
//! ```
//! `GLOT_REAL_SECRETS_FILE` reads the JSON from a file instead; `GLOT_REAL_SETTINGS` (JSON) and
//! `GLOT_REAL_LOCALE` (default `fr`) are optional. A local instance (http://127.0.0.1:…) also needs
//! `GLOT_CONNECTORS_ALLOW_PRIVATE=1`.
use glot_server::connectors::{Creds, PushItem, registry};
use serde_json::Value;

fn short(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(70).collect()
}

#[tokio::test]
#[ignore = "needs a real platform: set GLOT_REAL_KIND and GLOT_REAL_SECRETS"]
async fn doctor_against_a_real_platform() {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let kind = env("GLOT_REAL_KIND").expect("GLOT_REAL_KIND (shopify, wordpress, …)");
    let secrets = env("GLOT_REAL_SECRETS")
        .or_else(|| {
            env("GLOT_REAL_SECRETS_FILE").map(|f| std::fs::read_to_string(f).expect("secrets file"))
        })
        .expect("GLOT_REAL_SECRETS or GLOT_REAL_SECRETS_FILE (JSON object)");
    let secrets: Value = serde_json::from_str(&secrets).expect("secrets must be a JSON object");
    let settings: Value = env("GLOT_REAL_SETTINGS")
        .map(|s| serde_json::from_str(&s).expect("settings must be JSON"))
        .unwrap_or_else(|| serde_json::json!({}));
    let locale = env("GLOT_REAL_LOCALE").unwrap_or_else(|| "fr".into());
    let c = registry(&kind).unwrap_or_else(|| panic!("unknown connector '{kind}'"));
    // Same preparation as a dashboard connection (a Shopify Dev Dashboard app gets its first token here).
    let secrets = glot_server::connectors::oauth::prepare(&kind, secrets)
        .await
        .unwrap_or_else(|e| panic!("FAIL auth: {e:#}"));
    let creds = Creds {
        secrets,
        settings,
        since: None,
    };

    let info = c
        .test(&creds)
        .await
        .unwrap_or_else(|e| panic!("FAIL auth: {e:#}"));
    println!(
        "ok   auth: {} (source language: {:?})",
        info.name, info.source_locale
    );
    for n in &info.notes {
        println!("note {n}");
    }
    match c.locales(&creds).await {
        Ok(l) => println!("ok   languages: {}", l.join(", ")),
        Err(e) => println!("FAIL languages: {e:#}"),
    }
    match c.pull(&creds, None).await {
        Ok(page) => {
            println!(
                "ok   read: {} translatable field(s) on the first page, more pages: {}",
                page.items.len(),
                page.next.is_some()
            );
            for i in page.items.iter().take(8) {
                println!("       {} [{}] {}", i.key, i.format.name(), short(&i.text));
            }
            let probe: Vec<PushItem> = page
                .items
                .iter()
                .take(5)
                .map(|i| PushItem {
                    external_id: i.external_id.clone(),
                    field: i.field.clone(),
                    value: i.text.clone(),
                    format: i.format.clone(),
                    meta: i.meta.clone(),
                })
                .collect();
            match c.current(&creds, &locale, &probe).await {
                Ok(Some(cur)) => println!(
                    "ok   translations ({locale}) can be read back: {} of {} probed fields already translated",
                    cur.iter().flatten().count(),
                    probe.len()
                ),
                Ok(None) => println!(
                    "warn translations cannot be read back on this platform: no human-edit protection, no rollback"
                ),
                Err(e) => println!("FAIL reading translations ({locale}): {e:#}"),
            }
        }
        Err(e) => println!("FAIL read: {e:#}"),
    }
    for check in c.doctor(&creds).await {
        println!(
            "{:<4} {}: {}{}",
            check.status,
            check.id,
            check.detail,
            check
                .fix
                .map(|f| format!(" [fix: {f}]"))
                .unwrap_or_default()
        );
    }
}
