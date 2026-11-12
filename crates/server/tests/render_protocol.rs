//! Render protocol (`POST /v1/render`): markup and variable preservation, deterministic memory, safe fallback,
//! privacy (what the provider receives), quotas, idempotency.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable. The LLM is a local OpenAI-compatible
//! server that records every request body and answers with glot's deterministic mock (`⟦fr⟧ source`).
use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::post,
};
use glot_llm::provider::{CompletionRequest, Provider};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
use tower::ServiceExt;

const OK: u8 = 0;
const DOWN: u8 = 1;
const BREAKS_PLACEHOLDERS: u8 = 2;

struct Llm {
    base: String,
    /// Every request body the provider received, verbatim.
    seen: Arc<Mutex<Vec<String>>>,
    mode: Arc<AtomicU8>,
}

impl Llm {
    fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    fn received(&self) -> String {
        self.seen.lock().unwrap().join("\n")
    }
}

async fn fake_llm() -> Llm {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let mode = Arc::new(AtomicU8::new(OK));
    let (s, m) = (seen.clone(), mode.clone());
    let app = Router::new().route(
        "/chat/completions",
        post(move |body: String| async move {
            s.lock().unwrap().push(body.clone());
            let b: Value = serde_json::from_str(&body).unwrap();
            let text = |v: &Value| match v {
                Value::String(s) => s.clone(),
                v => v[0]["text"].as_str().unwrap_or_default().to_string(),
            };
            let mode = m.load(Ordering::SeqCst);
            if mode == DOWN {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": {"message": "model overloaded"}})),
                );
            }
            let c = glot_llm::mock::MockProvider
                .complete(CompletionRequest {
                    model: "fake".into(),
                    system: text(&b["messages"][0]["content"]),
                    user: text(&b["messages"][1]["content"]),
                    max_tokens: 1000,
                    json: true,
                    images: vec![],
                })
                .await
                .unwrap();
            let mut out = c.text;
            if mode == BREAKS_PLACEHOLDERS {
                out = out
                    .replace("{v0}", "")
                    .replace("<0>", "")
                    .replace("{name}", "{nom}");
            }
            (
                StatusCode::OK,
                Json(
                    json!({"model": "fake", "choices": [{"message": {"content": out}}],
                            "usage": {"prompt_tokens": 10, "completion_tokens": 10}}),
                ),
            )
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Llm { base, seen, mode }
}

struct Env {
    app: Router,
    admin: PgPool,
    pool: PgPool,
    db: String,
    cookie: String,
    key: String,
    llm: Llm,
}

impl Env {
    async fn call(
        &self,
        method: Method,
        uri: &str,
        auth: Auth<'_>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        match auth {
            Auth::Cookie => {
                req = req.header(header::COOKIE, format!("glot_session={}", self.cookie))
            }
            Auth::Key(k) => req = req.header(header::AUTHORIZATION, format!("Bearer {k}")),
            Auth::Idem(k, i) => {
                req = req
                    .header(header::AUTHORIZATION, format!("Bearer {k}"))
                    .header("idempotency-key", i)
            }
            Auth::None => {}
        }
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn render(&self, body: Value) -> Value {
        let (s, _, v) = self
            .call(Method::POST, "/v1/render", Auth::Key(&self.key), Some(body))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    async fn new_key(&self, body: Value) -> String {
        let (s, _, v) = self
            .call(
                Method::POST,
                "/api/projects/rp/render/keys",
                Auth::Cookie,
                Some(body),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["key"].as_str().unwrap().to_string()
    }

    async fn drop(self) {
        self.pool.close().await;
        let _ = sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.db))
            .execute(&self.admin)
            .await;
    }
}

enum Auth<'a> {
    None,
    Cookie,
    Key(&'a str),
    Idem(&'a str, &'a str),
}

async fn env(billing: Option<glot_server::billing::Config>) -> Option<Env> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return None;
    };
    let db = format!("glot_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {db}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPool::connect_with(url.parse::<PgConnectOptions>().unwrap().database(&db))
        .await
        .unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let mut state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    state.billing = billing.map(Arc::new);
    let app = glot_server::router(state, "http://localhost:3100");
    let llm = fake_llm().await;
    let mut e = Env {
        app,
        admin,
        pool,
        db,
        cookie: String::new(),
        key: String::new(),
        llm,
    };

    let (s, h, _) = e
        .call(
            Method::POST,
            "/api/auth/signup",
            Auth::None,
            Some(json!({"email": "r@x.io", "password": "correct horse", "name": "R"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    e.cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    let (s, _, _) = e
        .call(
            Method::POST,
            "/api/projects",
            Auth::Cookie,
            Some(
                json!({"name": "R", "slug": "rp", "source_locale": "en", "target_locales": ["fr"]}),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _, v) = e
        .call(Method::PATCH, "/api/projects/rp", Auth::Cookie,
              Some(json!({"llm": {"provider": "openai-compatible", "model": "fake", "base_url": e.llm.base}})))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    e.key = e.new_key(json!({"name": "backend"})).await;
    assert!(e.key.starts_with("glot_rk_"));
    Some(e)
}

const EMAIL_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Your order has shipped</title>
<style>.btn{background:#111;color:#fff} /* a { b } <not a tag> */</style></head>
<body style="margin:0">
<!--[if mso]><table><tr><td><![endif]-->
<span style="display:none">Track your parcel &amp; get ready</span>
<table role="presentation" width="100%"><tr><td align="center">
  <img src="https://cdn.acme.io/logo.png?v=3&amp;w=120" alt="Acme logo" width="120">
  <h1 style="font:600 22px Arial">Hi {{first_name}}, it&rsquo;s on the way!</h1>
  <p>Your order <b>#48213</b> left our warehouse. Questions? Write to <a href="mailto:help@acme.io">help@acme.io</a>
     or read <a href="https://acme.io/help?utm_source=email&amp;utm_campaign=ship" target="_blank">our <i>shipping</i> guide</a>.</p>
  <p>Total paid: {{total}}<br>Estimated delivery: {{eta}}</p>
  <a class="btn" href="{{tracking_url}}">Track my parcel</a>
  {{#if gift}}<p>This order is a gift.</p>{{/if}}
</td></tr></table>
<p style="font-size:11px">You receive this email because you ordered at Acme.
   <a href="{{{unsubscribe_url}}}">Unsubscribe</a></p>
<!--[if mso]></td></tr></table><![endif]-->
</body></html>"#;

fn email_body() -> Value {
    json!({
        "locale": "fr-FR",
        "type": "email",
        "context": "Shipping confirmation email of an online shop",
        "content": {
            "subject": "{{first_name}}, your order has shipped",
            "html": EMAIL_HTML,
            "text": "Hi {{first_name}},\n\nYour order #48213 left our warehouse.\nTrack it: {{tracking_url}}\n\nTotal paid: {{total}}\n"
        },
        "variables": {
            "first_name": "Zoë <Lovelace>",
            "total": {"type": "currency", "value": 1234.5, "currency": "EUR"},
            "eta": {"type": "date", "value": "2026-11-14", "style": "long"},
            "tracking_url": "https://track.acme.io/p/SECRET-TOKEN-9f3a?x=1&y=2",
            "unsubscribe_url": "https://acme.io/u/PRIVATE-77"
        }
    })
}

/// Names and attributes of every tag, in document order.
fn tags(html: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        let Some(j) = rest[i..].find('>') else { break };
        out.push(rest[i..=i + j].to_string());
        rest = &rest[i + j + 1..];
    }
    out
}

#[tokio::test]
async fn html_email_keeps_markup_fills_variables_and_never_shows_them_to_the_provider() {
    let Some(e) = env(None).await else { return };
    let v = e.render(email_body()).await;
    assert_eq!(v["status"], "translated", "{v}");
    assert_eq!(v["cache"], "miss");
    let html = v["content"]["html"].as_str().unwrap();

    // Translated: every visible sentence went through the model.
    for s in [
        "<title>⟦fr-FR⟧ Your order has shipped</title>",
        "alt=\"⟦fr-FR⟧ Acme logo\"",
        "⟦fr-FR⟧ Hi Zoë",
        "⟦fr-FR⟧ You receive this email",
        "Track my parcel</a>",
    ] {
        assert!(html.contains(s), "missing {s:?} in {html}");
    }
    // Markup: same tags with the same attributes, in the same order, except where a variable or a translated
    // attribute sits. CSS, conditional comments, entities in URLs, block helpers: byte for byte.
    let (src, out) = (tags(EMAIL_HTML), tags(html));
    assert_eq!(src.len(), out.len(), "{out:#?}");
    for (a, b) in src.iter().zip(&out) {
        if !a.contains("{{") && !a.contains("alt=") {
            assert_eq!(a, b);
        }
    }
    for kept in [
        ".btn{background:#111;color:#fff} /* a { b } <not a tag> */",
        "<!--[if mso]><table><tr><td><![endif]-->",
        r#"src="https://cdn.acme.io/logo.png?v=3&amp;w=120""#,
        r#"href="https://acme.io/help?utm_source=email&amp;utm_campaign=ship" target="_blank""#,
        r#"href="mailto:help@acme.io">help@acme.io</a>"#,
        "<b>#48213</b>",
        "{{#if gift}}",
        "{{/if}}",
    ] {
        assert!(html.contains(kept), "lost {kept:?} in {html}");
    }
    // Variables: escaped in text and attributes, raw with triple braces, formatted for fr-FR.
    assert!(html.contains("Zoë &lt;Lovelace&gt;"), "{html}");
    assert!(
        html.contains(r#"href="https://track.acme.io/p/SECRET-TOKEN-9f3a?x=1&amp;y=2""#),
        "{html}"
    );
    assert!(html.contains(r#"href="https://acme.io/u/PRIVATE-77""#));
    assert!(html.contains("1\u{202f}234,50\u{a0}€"), "{html}");
    assert!(html.contains("14 novembre 2026"), "{html}");
    assert!(!html.contains("{{first_name}}") && !html.contains("{{total}}"));
    // Subject and plain text: no HTML escaping there.
    assert_eq!(
        v["content"]["subject"],
        "⟦fr-FR⟧ Zoë <Lovelace>, your order has shipped"
    );
    let text = v["content"]["text"].as_str().unwrap();
    assert!(
        text.starts_with(
            "⟦fr-FR⟧ Hi Zoë <Lovelace>,\n\n⟦fr-FR⟧ Your order #48213 left our warehouse.\n"
        ),
        "{text}"
    );
    assert!(text.contains("https://track.acme.io/p/SECRET-TOKEN-9f3a?x=1&y=2\n\n"));

    // Privacy: the provider was called, and nothing personal is in what it received. Variable values, variable
    // names, and the values written in the content itself (order number, email address, URLs).
    assert!(e.llm.calls() > 0);
    let sent = e.llm.received();
    assert!(
        sent.contains("Your order has shipped") && sent.contains("{v0}"),
        "{sent}"
    );
    for secret in [
        "Zoë",
        "Lovelace",
        "1234",
        "2026-11-14",
        "SECRET-TOKEN",
        "PRIVATE-77",
        "track.acme.io",
        "first_name",
        "unsubscribe_url",
        "48213",
        "help@acme.io",
        "utm_campaign",
    ] {
        assert!(!sent.contains(secret), "{secret:?} reached the provider");
    }
    // Nothing personal is stored either.
    let stored: Vec<(String, String)> = sqlx::query_as("SELECT source, value FROM render_tm")
        .fetch_all(&e.pool)
        .await
        .unwrap();
    assert!(stored.len() >= 8);
    for (s, t) in &stored {
        assert!(
            !s.contains("Lovelace")
                && !t.contains("Lovelace")
                && !s.contains("48213")
                && !t.contains("SECRET")
        );
    }
    e.drop().await;
}

#[tokio::test]
async fn the_same_content_never_pays_twice() {
    let Some(e) = env(None).await else { return };
    let first = e.render(email_body()).await;
    let calls = e.llm.calls();
    assert!(calls > 0);

    let again = e.render(email_body()).await;
    assert_eq!(e.llm.calls(), calls, "a known document called the model");
    assert_eq!(again["cache"], "hit");
    assert_eq!(again["content"], first["content"], "not deterministic");
    assert_eq!(again["units"]["translated"], 0);

    // Another customer, another order number in the text: same templates, still no model call.
    let mut other = email_body();
    other["variables"]["first_name"] = json!("Kenji");
    other["content"]["text"] = json!(
        "Hi {{first_name}},\n\nYour order #99001 left our warehouse.\nTrack it: {{tracking_url}}\n\nTotal paid: {{total}}\n"
    );
    let v = e.render(other).await;
    assert_eq!(
        (e.llm.calls(), v["cache"].as_str()),
        (calls, Some("hit")),
        "{v}"
    );
    assert!(v["content"]["text"].as_str().unwrap().contains("#99001"));
    assert!(v["content"]["html"].as_str().unwrap().contains("Hi Kenji"));

    // One paragraph changes: only that paragraph is translated.
    let v = e
        .render(json!({"locale": "fr-FR", "type": "text",
                       "content": "Hi {{first_name}},\n\nIt will arrive on a pallet."}))
        .await;
    assert_eq!(v["cache"], "partial", "{v}");
    assert_eq!(
        v["units"],
        json!({"total": 2, "cached": 1, "translated": 1, "fallback": 0})
    );
    assert_eq!(e.llm.calls(), calls + 1);

    // The dashboard sees it: hit rate from stored counters.
    let (_, _, u) = e
        .call(
            Method::GET,
            "/api/projects/rp/render/usage",
            Auth::Cookie,
            None,
        )
        .await;
    assert_eq!(u["totals"]["requests"], 4, "{u}");
    let (units, cached) = (
        u["totals"]["units"].as_f64().unwrap(),
        u["totals"]["cached"].as_f64().unwrap(),
    );
    assert!((u["hit_rate"].as_f64().unwrap() - cached / units).abs() < 1e-9 && cached > 0.0);
    assert_eq!(u["memory_locales"], 1);
    e.drop().await;
}

#[tokio::test]
async fn a_failing_provider_or_a_broken_translation_falls_back_to_the_source() {
    let Some(e) = env(None).await else { return };
    let body = json!({
        "locale": "fr", "type": "html",
        "content": "<p>Hello <b>{{name}}</b>, your invoice is <a href=\"{{url}}\">ready</a>.</p>",
        "variables": {"name": "Ada & Co", "url": "https://acme.io/i/1"}
    });
    let source_filled = "<p>Hello <b>Ada &amp; Co</b>, your invoice is <a href=\"https://acme.io/i/1\">ready</a>.</p>";

    // Provider down: 200, the source document, variables filled, and the reason.
    e.llm.mode.store(DOWN, Ordering::SeqCst);
    let v = e.render(body.clone()).await;
    assert_eq!(v["status"], "fallback", "{v}");
    assert_eq!(v["content"], source_filled);
    assert!(
        v["fallback_reason"]
            .as_str()
            .unwrap()
            .starts_with("provider_error:"),
        "{v}"
    );
    assert_eq!(v["units"]["fallback"], 1);

    // The model drops a placeholder and a tag on every attempt: refused, never returned, never remembered.
    e.llm.mode.store(BREAKS_PLACEHOLDERS, Ordering::SeqCst);
    let v = e.render(body.clone()).await;
    assert_eq!(v["status"], "fallback", "{v}");
    assert_eq!(v["content"], source_filled);
    assert!(
        v["fallback_reason"]
            .as_str()
            .unwrap()
            .starts_with("validation:"),
        "{v}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM render_tm")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // Same for an ICU template whose variable the model renames.
    let v = e
        .render(json!({"locale": "fr", "type": "template", "content": "Welcome {name}!", "variables": {"name": "Ada"}}))
        .await;
    assert_eq!(
        (v["status"].as_str(), v["content"].as_str()),
        (Some("fallback"), Some("Welcome Ada!")),
        "{v}"
    );

    // An email is all or nothing: a subject in French over a body in English is not an answer.
    e.llm.mode.store(OK, Ordering::SeqCst);
    e.render(json!({"locale": "fr", "type": "text", "content": "Your invoice"}))
        .await;
    let cached = e
        .new_key(json!({"name": "edge", "scopes": ["render:cached"]}))
        .await;
    let mail = json!({"locale": "fr", "type": "email", "content": {"subject": "Your invoice", "text": "Never seen before."}});
    let (s, _, v) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Key(&cached),
            Some(mail.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["status"], "fallback", "{v}");
    assert_eq!(v["content"]["subject"], "Your invoice");
    assert!(
        v["fallback_reason"]
            .as_str()
            .unwrap()
            .starts_with("not_cached:")
    );
    // …unless the caller accepts a partial document.
    let mut partial = mail;
    partial["allow_partial"] = json!(true);
    let (_, _, v) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Key(&cached),
            Some(partial),
        )
        .await;
    assert_eq!(v["status"], "partial", "{v}");
    assert_eq!(
        v["content"],
        json!({"subject": "⟦fr⟧ Your invoice", "text": "Never seen before."})
    );

    // Once the provider answers, the first document is translated: fallbacks were not remembered.
    let v = e.render(body).await;
    assert_eq!(v["status"], "translated", "{v}");
    assert_eq!(
        v["content"],
        "<p>⟦fr⟧ Hello <b>Ada &amp; Co</b>, your invoice is <a href=\"https://acme.io/i/1\">ready</a>.</p>"
    );
    e.drop().await;
}

#[tokio::test]
async fn typed_content_templates_json_markdown_and_batch() {
    let Some(e) = env(None).await else { return };
    // Glossary and catalog are the project's: a catalog translation of the same source is reused without a
    // model call, a glossary term is imposed on new translations.
    let (s, _, _) = e
        .call(
            Method::POST,
            "/api/projects/rp/keys",
            Auth::Cookie,
            Some(json!({"key": "home.back", "source": "Welcome back"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _, v) = e
        .call(
            Method::PUT,
            "/api/projects/rp/keys/home.back/translations/fr",
            Auth::Cookie,
            Some(json!({"value": "Bon retour"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _, v) = e
        .call(
            Method::POST,
            "/api/projects/rp/glossary",
            Auth::Cookie,
            Some(json!({"term": "workspace", "translations": {"fr": "espace de travail"}})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let (s, _, v) = e
        .call(Method::POST, "/v1/render/batch", Auth::Key(&e.key), Some(json!({"items": [
            {"id": "push", "locale": "fr", "type": "text", "content": "Welcome back"},
            {"id": "tpl", "locale": "fr", "type": "template",
             "content": "{name} shared {count, plural, one {# file} other {# files}} in your workspace, total {amount}",
             "variables": {"name": "Ada", "count": 1234, "amount": {"type": "currency", "value": 9.9, "currency": "USD"}}},
            {"id": "doc", "locale": "fr", "type": "json",
             "fields": ["title", "lines.*.label", {"path": "note", "type": "markdown"}],
             "content": {"id": "inv_42", "currency": "EUR", "title": "Invoice",
                         "lines": [{"label": "Yearly plan", "sku": "PLAN-Y", "amount": 1200}],
                         "note": "Paid by **card**. See [terms](https://acme.io/terms)."}},
            {"id": "same", "locale": "en", "type": "text", "content": "Hi {{name}}", "variables": {"name": "Bob"}},
            {"id": "bad", "locale": "fr", "type": "template", "content": "{count, plural, one {# file}"},
            {"id": "missing-var", "locale": "fr", "type": "template", "content": "Hi {name}", "variables": {}},
            {"id": "bad-date", "locale": "fr", "content": "x", "variables": {"d": {"type": "date", "value": "soon"}}}
        ]})))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let by = |id: &str| {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .unwrap()
            .clone()
    };

    assert_eq!(by("push")["content"], "Bon retour"); // the catalog's human translation, not the model's
    assert_eq!(by("push")["cache"], "hit");
    // Plural chosen and number grouped for French, glossary term imposed, currency formatted.
    assert_eq!(
        by("tpl")["content"],
        "⟦fr⟧ Ada shared 1\u{202f}234 files in your espace de travail, total 9,90\u{a0}$"
    );
    let doc = by("doc");
    assert_eq!(doc["status"], "translated", "{doc}");
    assert_eq!(
        doc["content"],
        json!({"id": "inv_42", "currency": "EUR", "title": "⟦fr⟧ Invoice",
               "lines": [{"label": "⟦fr⟧ Yearly plan", "sku": "PLAN-Y", "amount": 1200}],
               "note": "⟦fr⟧ Paid by **card**. See [terms](https://acme.io/terms)."})
    );
    assert_eq!(
        (
            by("same")["status"].as_str(),
            by("same")["content"].as_str()
        ),
        (Some("source"), Some("Hi Bob"))
    );
    // Every refusal says what to fix.
    assert!(
        by("bad")["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ICU")
    );
    assert!(
        by("missing-var")["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing variable 'name'")
    );
    assert!(
        by("bad-date")["error"]["message"]
            .as_str()
            .unwrap()
            .contains("variable 'd'")
    );
    // The catalog hit and the invalid items never reached the model.
    let sent = e.llm.received();
    assert!(
        !sent.contains("Welcome back") && !sent.contains("Ada") && !sent.contains("1234"),
        "{sent}"
    );

    // The dashboard playground runs the same pipeline with the member's session.
    let (s, _, v) = e
        .call(
            Method::POST,
            "/api/projects/rp/render",
            Auth::Cookie,
            Some(json!({"items": [{"locale": "fr", "type": "text", "content": "Welcome back"}]})),
        )
        .await;
    assert_eq!(
        (s, v["items"][0]["content"].as_str()),
        (StatusCode::OK, Some("Bon retour"))
    );
    e.drop().await;
}

#[tokio::test]
async fn keys_scopes_quotas_and_idempotency() {
    let mut cfg = glot_server::billing::Config::new(
        "sk_test_x",
        "whsec_x",
        "http://localhost:1",
        "http://localhost:3100",
    );
    cfg.plans.get_mut("free").unwrap().render_requests = Some(6);
    let Some(e) = env(Some(cfg)).await else {
        return;
    };
    let body = |s: &str| json!({"locale": "fr", "type": "text", "content": s});

    // No key, a wrong key, an account token: 401 with what to send.
    for auth in [
        Auth::None,
        Auth::Key("glot_rk_nope"),
        Auth::Key("glot_0123"),
    ] {
        let (s, _, v) = e
            .call(Method::POST, "/v1/render", auth, Some(body("Hello")))
            .await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("render key"),
            "{v}"
        );
    }
    // Only the hash is stored, and the list never shows the key again.
    let (_, _, list) = e
        .call(
            Method::GET,
            "/api/projects/rp/render/keys",
            Auth::Cookie,
            None,
        )
        .await;
    assert!(
        list[0].get("key").is_none() && list[0]["prefix"].as_str().unwrap().starts_with("glot_rk_")
    );
    let hashes: Vec<String> = sqlx::query_scalar("SELECT key_hash FROM render_keys")
        .fetch_all(&e.pool)
        .await
        .unwrap();
    assert!(!hashes.contains(&e.key));

    // Idempotency: the same key and body are answered from the stored answer, not counted, not translated again.
    let (s, h, first) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Idem(&e.key, "send-42"),
            Some(body("Hello there")),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.get("idempotent-replayed").is_none());
    let calls = e.llm.calls();
    let (s, h, replay) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Idem(&e.key, "send-42"),
            Some(body("Hello there")),
        )
        .await;
    assert_eq!((s, &replay), (StatusCode::OK, &first));
    assert_eq!(h["idempotent-replayed"], "true");
    assert_eq!(e.llm.calls(), calls);
    let used = || async {
        sqlx::query_scalar::<_, i64>("SELECT coalesce(sum(requests), 0)::int8 FROM render_usage")
            .fetch_one(&e.pool)
            .await
            .unwrap()
    };
    assert_eq!(used().await, 1);
    let (s, _, v) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Idem(&e.key, "send-42"),
            Some(body("Something else")),
        )
        .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("idempotency_key_reused")
        )
    );

    // A key's own monthly limit.
    let small = e.new_key(json!({"name": "cron", "monthly_limit": 2})).await;
    for _ in 0..2 {
        let (s, _, _) = e
            .call(
                Method::POST,
                "/v1/render",
                Auth::Key(&small),
                Some(body("Hello there")),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, _, v) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Key(&small),
            Some(body("Hello there")),
        )
        .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("key_quota_exceeded")),
        "{v}"
    );
    assert!(v["error"]["message"].as_str().unwrap().contains("2 of 2"));

    // The plan's monthly quota (billing.rs): 6 documents on this test plan, 3 used so far.
    for _ in 0..3 {
        let (s, _, v) = e
            .call(
                Method::POST,
                "/v1/render",
                Auth::Key(&e.key),
                Some(body("Hello there")),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let (s, _, v) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Key(&e.key),
            Some(body("Hello there")),
        )
        .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (StatusCode::PAYMENT_REQUIRED, Some("quota_exceeded")),
        "{v}"
    );
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("render API")
    );

    // A revoked key stops working at once.
    let id = list[0]["id"].as_str().unwrap();
    let (s, _, _) = e
        .call(
            Method::DELETE,
            &format!("/api/projects/rp/render/keys/{id}"),
            Auth::Cookie,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = e
        .call(
            Method::POST,
            "/v1/render",
            Auth::Key(&e.key),
            Some(body("Hello")),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    e.drop().await;
}

/// Not a test: runs the pipeline on a copy of a real database and prints what it gives.
/// `RENDER_REAL_DB=postgres://…/glot_real_render_protocol RENDER_REAL_BODY=body.json RENDER_REAL_PROJECT=cliponit
///  cargo test -p glot-server --test render_protocol real_project -- --ignored --nocapture`
/// The database must already have migration 0180. Every item is forced to `cache_only`: no model is ever called.
#[tokio::test]
#[ignore]
async fn real_project() {
    let (Ok(url), Ok(body), Ok(slug)) = (
        std::env::var("RENDER_REAL_DB"),
        std::env::var("RENDER_REAL_BODY"),
        std::env::var("RENDER_REAL_PROJECT"),
    ) else {
        eprintln!("SKIPPED: set RENDER_REAL_DB, RENDER_REAL_BODY and RENDER_REAL_PROJECT");
        return;
    };
    let pool = PgPool::connect(&url).await.unwrap();
    let key = format!("glot_rk_{}", uuid::Uuid::new_v4().simple());
    use sha2::Digest;
    sqlx::query(
        "INSERT INTO render_keys (id, project_id, name, prefix, key_hash, scopes)
         SELECT $1, id, 'real-data run', 'glot_rk_', $2, '{render:cached}' FROM projects WHERE slug = $3",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(hex::encode(sha2::Sha256::digest(key.as_bytes())))
    .bind(&slug)
    .execute(&pool)
    .await
    .unwrap();
    let app = glot_server::router(
        glot_server::AppState::new(pool, "real-data", false),
        "http://localhost:3100",
    );
    let mut body: Value = serde_json::from_str(&std::fs::read_to_string(body).unwrap()).unwrap();
    for i in body["items"].as_array_mut().unwrap() {
        i["cache_only"] = json!(true);
    }
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/render/batch")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
}
