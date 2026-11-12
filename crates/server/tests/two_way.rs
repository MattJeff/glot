//! Two-way translation. Core with a recording provider (what the model is sent), then the HTTP surface on a
//! real Postgres (DATABASE_URL, default :5433; skipped when unreachable) with the mock provider, and the
//! Help Scout adapter against a local fake of the Mailbox API.
use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, Method, Request, StatusCode, Uri, header},
    response::IntoResponse,
};
use glot_core::glossary::{Glossary, Term};
use glot_llm::provider::{Completion, CompletionRequest, Provider};
use glot_server::two_way::{self, Engine, helpscout};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

// ------------------------------------------------------------------------------------------------ core

/// The mock provider, recording every prompt; `diverge` answers back-translations with something unrelated.
struct Recording {
    seen: Arc<Mutex<Vec<String>>>,
    diverge: bool,
}

#[async_trait]
impl Provider for Recording {
    fn name(&self) -> &str {
        "recording"
    }
    async fn complete(&self, req: CompletionRequest) -> glot_llm::Result<Completion> {
        self.seen
            .lock()
            .unwrap()
            .push(format!("{}\n{}", req.system, req.user));
        if self.diverge && req.user.contains("Back-translation used as a safety check") {
            return Ok(Completion {
                text: json!({"translations": {"message": "Nous ne pouvons rien faire, voyez avec le transporteur."}})
                    .to_string(),
                model: "mock".into(),
                ..Default::default()
            });
        }
        glot_llm::mock::MockProvider.complete(req).await
    }
}

fn engine(diverge: bool) -> (Engine, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let cfg: glot_core::config::Config = toml::from_str(
        "source_locale = 'en'\ntarget_locales = ['fr', 'ja']\nfiles = []\n[llm]\nprovider = 'mock'\nmodel = 'mock'\n",
    )
    .unwrap();
    let provider = Recording {
        seen: seen.clone(),
        diverge,
    };
    let term = |term: &str, tr: &[(&str, &str)], dnt: bool| Term {
        term: term.into(),
        description: None,
        do_not_translate: dnt,
        case_sensitive: false,
        translations: tr
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        forbidden: Default::default(),
    };
    let e = Engine {
        translator: glot_llm::translate::Translator::new(&cfg, Box::new(provider)),
        glossary: Glossary {
            terms: vec![term(
                "workspace",
                &[("fr", "espace de travail"), ("ja", "ワークスペース")],
                false,
            )],
        },
        project_source: "en".into(),
        style: Some("Vouvoiement, ton chaleureux.".into()),
    };
    (e, seen)
}

const SECRETS: [&str; 6] = [
    "tanaka.yuki@example.jp",
    "090-1234-5678",
    "48213",
    "4242 4242 4242 4242",
    "ORD-2026-00917",
    "old.address@example.jp",
];

#[tokio::test]
async fn pii_never_reaches_the_provider_and_glossary_applies_both_ways() {
    let (e, seen) = engine(false);
    let text = "ワークスペースが開けません!!! 注文番号は#48213とORD-2026-00917です。至急、tanaka.yuki@example.jp か 090-1234-5678 に連絡してください。カードは 4242 4242 4242 4242 です。";
    let thread = vec![(
        "customer".to_string(),
        "前のメールは old.address@example.jp から送りました。".to_string(),
    )];
    let r = two_way::inbound(&e, text, "fr", None, None, 0.5, &thread)
        .await
        .unwrap();
    assert_eq!(r.locale.as_deref(), Some("ja"));
    assert!(r.confident);
    assert_eq!(r.pii_masked, 5);
    assert!(r.flags.is_empty(), "{:?}", r.flags);
    let tr = r.translation.unwrap();
    // Originals restored for the team, glossary pivoted ja → fr (the project's source is en).
    for s in &SECRETS[..5] {
        assert!(tr.contains(s), "{s} not restored in {tr}");
    }
    assert!(
        tr.contains("espace de travail") && !tr.contains("ワークスペース"),
        "{tr}"
    );
    // Emphasis is the customer's: nothing removed it.
    assert!(tr.contains("!!!"), "{tr}");

    // Outbound fr → ja, with its back-translation.
    let reply = "Bonjour, votre espace de travail est rouvert. Écrivez-nous à tanaka.yuki@example.jp si la commande #48213 pose encore problème.";
    let o = two_way::outbound(&e, reply, "fr", "ja", &thread, 0.6)
        .await
        .unwrap();
    assert!(
        o.translation.contains("ワークスペース"),
        "{}",
        o.translation
    );
    assert!(o.translation.contains("tanaka.yuki@example.jp") && o.translation.contains("#48213"));
    let back = o.back_translation.unwrap();
    assert!(
        back.contains("espace de travail") && back.contains("tanaka.yuki@example.jp"),
        "{back}"
    );
    assert!(
        o.confidence >= 0.6 && !o.needs_review,
        "{} {:?}",
        o.confidence,
        o.flags
    );
    assert_eq!(o.pii_masked, 2);

    let prompts = seen.lock().unwrap();
    assert_eq!(prompts.len(), 3, "inbound + outbound + back-translation");
    for p in prompts.iter() {
        for s in SECRETS {
            assert!(!p.contains(s), "{s} was sent to the provider:\n{p}");
        }
        // No fragment either (the local part of an address, the last digits of a card or a phone).
        for frag in ["tanaka", "example.jp", "4242", "5678", "00917"] {
            assert!(
                !p.contains(frag),
                "fragment {frag} was sent to the provider"
            );
        }
    }
    // The model is told to leave the customer's tone alone, and gets the style guide for the reply.
    assert!(prompts[0].contains("Never soften, summarize, explain or add anything"));
    assert!(prompts[0].contains("{email1}") && prompts[0].contains("{card1}"));
    assert!(prompts[1].contains("Vouvoiement"));
}

#[tokio::test]
async fn detection_thresholds_and_fallbacks() {
    let (e, seen) = engine(false);
    // Confident detection wins over the thread and the hint.
    let r = two_way::inbound(
        &e,
        "注文した商品がまだ届いていません。",
        "fr",
        Some("de"),
        Some("en"),
        0.5,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(r.locale.as_deref(), Some("ja"));
    // Too short to trust: the thread's language, then the hint, then a flagged guess.
    let r = two_way::inbound(&e, "ok danke", "fr", Some("de"), Some("en"), 0.5, &[])
        .await
        .unwrap();
    assert!(!r.confident && r.detect_confidence < 0.5);
    assert_eq!(r.locale.as_deref(), Some("de"));
    assert!(r.flags.is_empty() && r.translation.is_some());
    let r = two_way::inbound(&e, "ok danke", "fr", None, Some("de-AT"), 0.5, &[])
        .await
        .unwrap();
    assert_eq!(r.locale.as_deref(), Some("de"));
    let r = two_way::inbound(&e, "ok danke", "fr", None, None, 0.5, &[])
        .await
        .unwrap();
    assert!(
        r.flags.contains(&"language_uncertain".to_string()),
        "{:?}",
        r.flags
    );
    // A threshold of 0 trusts anything; nothing to detect is said, not guessed.
    let r = two_way::inbound(&e, "ok danke", "fr", Some("ja"), None, 0.0, &[])
        .await
        .unwrap();
    assert!(r.confident && r.locale.as_deref() != Some("ja"));
    let r = two_way::inbound(&e, "👍 12", "fr", None, None, 0.5, &[])
        .await
        .unwrap();
    assert_eq!(r.flags, vec!["untranslated:no_language".to_string()]);
    assert!(r.translation.is_none());

    // Already the team's language: no model call at all.
    let before = seen.lock().unwrap().len();
    let r = two_way::inbound(
        &e,
        "Bonjour, je voudrais modifier mon adresse de livraison.",
        "fr",
        None,
        None,
        0.5,
        &[],
    )
    .await
    .unwrap();
    assert!(r.translation.is_none() && r.flags.is_empty(), "{r:?}");
    assert_eq!(seen.lock().unwrap().len(), before);

    // Mixed: flagged, all of it translated, and the model is told.
    let r = two_way::inbound(
        &e,
        "Hallo, ich habe meine Bestellung noch nicht erhalten und möchte mein Geld zurück.\n注文した商品がまだ届いていません。至急ご確認ください、よろしくお願いします。",
        "fr",
        None,
        None,
        0.5,
        &[],
    )
    .await
    .unwrap();
    assert!(
        r.flags.iter().any(|f| f.starts_with("mixed:")),
        "{:?}",
        r.flags
    );
    assert!(r.translation.is_some());
    assert!(
        seen.lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("mixes several languages")
    );
}

#[tokio::test]
async fn divergent_back_translation_asks_for_review() {
    let (e, _) = engine(true);
    let o = two_way::outbound(
        &e,
        "Votre commande arrive demain, nous vous remboursons les frais de port.",
        "fr",
        "ja",
        &[],
        0.6,
    )
    .await
    .unwrap();
    assert!(o.confidence < 0.6, "{}", o.confidence);
    assert!(o.needs_review && o.flags == vec!["low_confidence".to_string()]);
    // Same language on both sides: nothing to translate, nothing to doubt.
    let o = two_way::outbound(&e, "Merci !", "fr", "fr-CA", &[], 0.6)
        .await
        .unwrap();
    assert!(o.back_translation.is_none() && !o.needs_review && o.translation == "Merci !");
}

// ------------------------------------------------------------------------------------------------ HTTP

struct App {
    app: Router,
    pool: PgPool,
    cookie: String,
}

impl App {
    async fn call(
        &self,
        method: Method,
        uri: &str,
        headers: &[(&str, String)],
        body: Option<String>,
    ) -> (StatusCode, HeaderMap, Value) {
        Self::send(&self.app, method, uri, headers, body).await
    }

    async fn send(
        app: &Router,
        method: Method,
        uri: &str,
        headers: &[(&str, String)],
        body: Option<String>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let res = app
            .clone()
            .oneshot(req.body(body.map_or(Body::empty(), Body::from)).unwrap())
            .await
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, headers, v)
    }

    /// Authenticated dashboard call.
    async fn api(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (s, _, v) = self
            .call(
                method,
                &format!("/api/projects/tw{path}"),
                &[("cookie", format!("glot_session={}", self.cookie))],
                body.map(|b| b.to_string()),
            )
            .await;
        (s, v)
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }
}

/// Fresh database + a project `tw` (source en, target fr, mock model, a glossary term), team language fr.
async fn setup() -> Option<(App, glot_server::AppState)> {
    // Local fakes (reply_url receiver, Help Scout) listen on 127.0.0.1.
    unsafe { std::env::set_var("GLOT_CONNECTORS_ALLOW_PRIVATE", "1") };
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
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state.clone(), "http://localhost:3100");
    let (s, h, _) = App::send(
        &app,
        Method::POST,
        "/api/auth/signup",
        &[],
        Some(json!({"email": "tw@x.io", "password": "correct horse", "name": "T"}).to_string()),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    let a = App { app, pool, cookie };
    let c = [("cookie", format!("glot_session={}", a.cookie))];
    let (s, _, _) = a
        .call(
            Method::POST,
            "/api/projects",
            &c,
            Some(json!({"name": "TW", "slug": "tw", "source_locale": "en", "target_locales": ["fr"]}).to_string()),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = a
        .api(
            Method::PATCH,
            "",
            Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = a
        .api(
            Method::POST,
            "/glossary",
            Some(json!({"term": "workspace", "translations": {"fr": "espace de travail", "ja": "ワークスペース"}})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, v) = a
        .api(
            Method::PUT,
            "/two-way/settings",
            Some(json!({"team_locale": "fr"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Zero configuration: suggest mode, nothing public.
    assert_eq!(v["mode"], "suggest");
    assert_eq!(v["form_enabled"], false);
    assert_eq!(v["has_webhook_secret"], false);
    Some((a, state))
}

fn signed(secret: &str, body: &str, t: i64) -> Vec<(&'static str, String)> {
    vec![(
        "glot-signature",
        glot_server::billing::sign(secret, body.as_bytes(), t),
    )]
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

const JA: &str =
    "ワークスペースが開けません。注文番号は#48213です。tanaka.yuki@example.jp に連絡してください。";

#[tokio::test]
async fn webhook_inbox_modes_and_stats() {
    let Some((a, _)) = setup().await else { return };
    let hook = "/api/two-way/tw/webhook";
    let body = json!({"conversation_id": "ticket-1", "message_id": "m1", "text": JA, "subject": "開けません"}).to_string();

    // No secret yet: the endpoint says what is missing. Then signatures are enforced.
    let (s, _, v) = a
        .call(
            Method::POST,
            hook,
            &signed("x", &body, now()),
            Some(body.clone()),
        )
        .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("not_configured"))
    );
    let (_, v) = a
        .api(
            Method::PUT,
            "/two-way/settings",
            Some(json!({"generate_webhook_secret": true})),
        )
        .await;
    let secret = v["webhook_secret"].as_str().unwrap().to_string();
    let (_, v) = a.api(Method::GET, "/two-way/settings", None).await;
    assert!(
        v["has_webhook_secret"] == true && v.get("webhook_secret").is_none(),
        "secret shown once"
    );

    for bad in [
        vec![],
        signed("wrong-secret", &body, now()),
        signed(&secret, "{\"other\":1}", now()),
        signed(&secret, &body, now() - 3600),
    ] {
        let (s, _, v) = a.call(Method::POST, hook, &bad, Some(body.clone())).await;
        assert_eq!(
            (s, v["error"]["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("bad_signature")),
            "{v}"
        );
    }
    assert_eq!(a.count("SELECT count(*) FROM two_way_messages").await, 0);

    let (s, _, m) = a
        .call(
            Method::POST,
            hook,
            &signed(&secret, &body, now()),
            Some(body.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["original"], JA, "the original is kept");
    assert_eq!(m["original_locale"], "ja");
    assert_eq!(m["translation_locale"], "fr");
    let tr = m["translation"].as_str().unwrap();
    assert!(
        tr.starts_with("⟦fr⟧") && tr.contains("espace de travail"),
        "{tr}"
    );
    assert!(
        tr.contains("tanaka.yuki@example.jp") && tr.contains("#48213"),
        "{tr}"
    );
    assert_eq!(m["pii_masked"], 2);
    assert_eq!(m["status"], "received");
    // A retried delivery is the same message.
    let (_, _, again) = a
        .call(
            Method::POST,
            hook,
            &signed(&secret, &body, now()),
            Some(body.clone()),
        )
        .await;
    assert_eq!(again["id"], m["id"]);
    assert_eq!(a.count("SELECT count(*) FROM two_way_messages").await, 1);

    // A short follow-up cannot be detected: the thread's language is used.
    let short =
        json!({"conversation_id": "ticket-1", "message_id": "m2", "text": "ok"}).to_string();
    let (_, _, m2) = a
        .call(
            Method::POST,
            hook,
            &signed(&secret, &short, now()),
            Some(short.clone()),
        )
        .await;
    assert_eq!(m2["original_locale"], "ja");
    assert_eq!(m2["flags"], json!([]));

    // REST (API token / session) is the same pipeline. Unknown short text: flagged. Team language: untouched.
    let (s, m3) = a
        .api(
            Method::POST,
            "/two-way/inbound",
            Some(json!({"conversation_id": "chat-9", "text": "ok danke"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{m3}");
    assert!(
        m3["flags"]
            .as_array()
            .unwrap()
            .contains(&json!("language_uncertain")),
        "{m3}"
    );
    let (_, m4) = a
        .api(
            Method::POST,
            "/two-way/inbound",
            Some(json!({"conversation_id": "chat-10", "text": "Bonjour, je voudrais modifier mon adresse de livraison."})),
        )
        .await;
    assert!(
        m4["translation"].is_null() && m4["original_locale"] == "fr",
        "{m4}"
    );

    // ---- inbox
    let (_, list) = a.api(Method::GET, "/two-way/conversations", None).await;
    let convs = list["conversations"].as_array().unwrap();
    assert_eq!(convs.len(), 3);
    let c1 = convs
        .iter()
        .find(|c| c["external_id"] == "ticket-1")
        .unwrap();
    assert_eq!(
        (c1["customer_locale"].as_str(), c1["messages"].as_i64()),
        (Some("ja"), Some(2))
    );
    let id = c1["id"].as_str().unwrap();
    let reply_path = format!("/two-way/conversations/{id}/reply");
    let reply = "Bonjour, votre espace de travail est rouvert. La commande #48213 est en route.";

    // Live preview: translation + back-translation, nothing stored.
    let (s, pv) = a
        .api(
            Method::POST,
            "/two-way/preview",
            Some(json!({"conversation_id": id, "text": reply})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{pv}");
    assert!(
        pv["translation"]
            .as_str()
            .unwrap()
            .contains("ワークスペース"),
        "{pv}"
    );
    assert!(
        pv["back_translation"]
            .as_str()
            .unwrap()
            .contains("espace de travail"),
        "{pv}"
    );
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_messages WHERE direction = 'out'")
            .await,
        0
    );

    // ---- suggest (default): translated, never sent, and sending is refused.
    let (s, d) = a
        .api(Method::POST, &reply_path, Some(json!({"text": reply})))
        .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(
        (d["status"].as_str(), d["sent_at"].as_str()),
        (Some("draft"), None)
    );
    assert!(
        d["translation"]
            .as_str()
            .unwrap()
            .contains("ワークスペース")
    );
    assert!(d["translation"].as_str().unwrap().contains("#48213"));
    assert_eq!(d["original"], reply);
    let (s, e) = a
        .api(
            Method::POST,
            &reply_path,
            Some(json!({"text": reply, "send": true})),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("mode_forbids_send"))
    );
    let send_path = format!("/two-way/messages/{}/send", d["id"].as_str().unwrap());
    let (s, e) = a.api(Method::POST, &send_path, None).await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("mode_forbids_send"))
    );
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_messages WHERE status = 'sent'")
            .await,
        0
    );

    // ---- one_click: still nothing leaves until a person sends; then it does, once.
    a.api(
        Method::PUT,
        "/two-way/settings",
        Some(json!({"mode": "one_click"})),
    )
    .await;
    let (_, d2) = a
        .api(Method::POST, &reply_path, Some(json!({"text": reply})))
        .await;
    assert_eq!(d2["status"], "draft");
    let (s, sent) = a.api(Method::POST, &send_path, None).await;
    assert_eq!(s, StatusCode::OK, "{sent}");
    assert_eq!(
        (sent["status"].as_str(), sent["delivered_via"].as_str()),
        (Some("sent"), Some("api"))
    );
    let (s, _) = a.api(Method::POST, &send_path, None).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // ---- auto: sent on its own, to the reply URL, signed; held back when confidence is under the bar.
    let got: Arc<Mutex<Vec<(String, String)>>> = Default::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let reply_url = format!("http://{}/replies", l.local_addr().unwrap());
    let sink = got.clone();
    tokio::spawn(async move {
        let app = Router::new().fallback(move |h: HeaderMap, b: Bytes| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push((
                    h["glot-signature"].to_str().unwrap().to_string(),
                    String::from_utf8(b.to_vec()).unwrap(),
                ));
                StatusCode::OK
            }
        });
        axum::serve(l, app).await.unwrap();
    });
    let (s, v) = a
        .api(
            Method::PUT,
            "/two-way/settings",
            Some(json!({"mode": "auto", "reply_url": reply_url, "review_below": 1.0})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, held) = a
        .api(Method::POST, &reply_path, Some(json!({"text": reply})))
        .await;
    assert_eq!(held["status"], "needs_review", "{held}");
    assert!(
        held["flags"]
            .as_array()
            .unwrap()
            .contains(&json!("low_confidence"))
    );
    assert!(
        got.lock().unwrap().is_empty(),
        "a flagged reply is not sent in auto mode"
    );

    a.api(
        Method::PUT,
        "/two-way/settings",
        Some(json!({"review_below": 0.5})),
    )
    .await;
    let (_, auto) = a
        .api(Method::POST, &reply_path, Some(json!({"text": reply})))
        .await;
    assert_eq!(
        (auto["status"].as_str(), auto["delivered_via"].as_str()),
        (Some("sent"), Some("webhook")),
        "{auto}"
    );
    let deliveries = got.lock().unwrap().clone();
    assert_eq!(deliveries.len(), 1);
    let (sig, payload) = &deliveries[0];
    glot_server::billing::verify_signature(&secret, sig, payload.as_bytes(), now()).unwrap();
    let payload: Value = serde_json::from_str(payload).unwrap();
    assert_eq!(payload["conversation_id"], "ticket-1");
    assert_eq!(payload["locale"], "ja");
    assert_eq!(payload["text"], auto["translation"]);
    assert_eq!(payload["original"], reply);
    // A private reply URL is refused when the test override is not what allowed it: the guard is the connectors'.
    let (s, e) = a
        .api(
            Method::PUT,
            "/two-way/settings",
            Some(json!({"reply_url": "ftp://example.com/x"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");

    // ---- a person corrects the language of the unsure conversation: translated again from it.
    let c9 = convs.iter().find(|c| c["external_id"] == "chat-9").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, fixed) = a
        .api(
            Method::PATCH,
            &format!("/two-way/conversations/{c9}"),
            Some(json!({"customer_locale": "de"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{fixed}");
    assert_eq!(fixed["customer_locale"], "de");
    assert_eq!(fixed["messages"][0]["original_locale"], "de");
    assert_eq!(fixed["messages"][0]["flags"], json!([]));

    // ---- language stats: real counts only; ja is not a language the project ships.
    let (_, st) = a.api(Method::GET, "/two-way/stats", None).await;
    assert_eq!(st["total"], 4, "{st}");
    let ja = st["languages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["locale"] == "ja")
        .unwrap();
    assert_eq!(
        (ja["messages"].as_i64(), ja["served"].as_bool()),
        (Some(2), Some(false))
    );
    let fr = st["languages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["locale"] == "fr")
        .unwrap();
    assert_eq!(fr["served"], true);
}

#[tokio::test]
async fn contact_form_and_inbound_email() {
    let Some((a, state)) = setup().await else {
        return;
    };
    let form = "/api/two-way/tw/form";
    let body = json!({"text": JA, "lang_hint": "ja-JP"}).to_string();
    let origin = [("origin", "https://shop.example".to_string())];

    // Off by default.
    let (s, _, v) = a
        .call(Method::POST, form, &origin, Some(body.clone()))
        .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("form_disabled"))
    );
    a.api(
        Method::PUT,
        "/two-way/settings",
        Some(json!({"form_enabled": true, "form_per_day": 1})),
    )
    .await;

    let (s, h, v) = a
        .call(Method::POST, form, &origin, Some(body.clone()))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h["access-control-allow-origin"], "*");
    assert_eq!(
        (v["language"].as_str(), v["team_locale"].as_str()),
        (Some("ja"), Some("fr"))
    );
    assert!(
        v["translation"]
            .as_str()
            .unwrap()
            .contains("espace de travail"),
        "{v}"
    );
    // Nothing personal stored: no conversation, no message, only the language counter.
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_conversations").await,
        0
    );
    assert_eq!(a.count("SELECT count(*) FROM two_way_messages").await, 0);
    assert_eq!(
        a.count("SELECT sum(messages)::bigint FROM two_way_lang_stats WHERE channel = 'form' AND locale = 'ja'").await,
        1
    );
    // Over the daily cap: the (free, local) language tag still comes back, no model call.
    let (s, _, v) = a
        .call(Method::POST, form, &origin, Some(body.clone()))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (v["language"].as_str(), v["reason"].as_str()),
        (Some("ja"), Some("daily_cap"))
    );
    assert!(v["translation"].is_null());
    // Opt-in storage puts form messages in the inbox.
    a.api(
        Method::PUT,
        "/two-way/settings",
        Some(json!({"form_store": true, "form_per_day": 50})),
    )
    .await;
    let (s, _, _) = a
        .call(Method::POST, form, &origin, Some(body.clone()))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_conversations WHERE channel = 'form'")
            .await,
        1
    );

    let (s, h, js) = a.call(Method::GET, "/api/two-way/form.js", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        h[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    assert!(js.as_str().unwrap().contains("glot_translation"));

    // ---- inbound email: same signature as the webhook; parse → translate → forward through the outbox.
    let (_, v) = a
        .api(
            Method::PUT,
            "/two-way/settings",
            Some(json!({"generate_webhook_secret": true, "forward_to": "support@shop.example"})),
        )
        .await;
    let secret = v["webhook_secret"].as_str().unwrap().to_string();
    let mut with_mail = state.clone();
    with_mail.mailer = Some(glot_server::email::Mailer::new(
        "re_test",
        "http://127.0.0.1:9",
        "glot <no-reply@glot.test>",
        "http://localhost:3100",
    ));
    let mail_app = glot_server::router(with_mail, "http://localhost:3100");
    let mail = |id: &str| {
        json!({
            "from": "Tanaka <tanaka.yuki@example.jp>",
            "subject": "Re: ご注文について",
            "message_id": id,
            "text": format!("{JA}\n\nOn Mon, 3 Nov 2026 at 10:02, Support <help@shop.example> wrote:\n> Bonjour,\n> votre colis est parti."),
        })
        .to_string()
    };
    // With email on, an owner who never confirmed their address spends nothing: the message is kept, and says why.
    let b = mail("<1@example.jp>");
    let (s, _, v) = App::send(
        &mail_app,
        Method::POST,
        "/api/two-way/tw/email",
        &signed(&secret, &b, now()),
        Some(b.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["message"]["translation"].is_null());
    assert!(
        v["message"]["flags"][0]
            .as_str()
            .unwrap()
            .starts_with("untranslated:"),
        "{v}"
    );
    assert!(v["message"]["error"].is_string());
    assert_eq!(
        v["message"]["original"], JA,
        "quoted history is dropped, the message is kept"
    );

    sqlx::query("UPDATE users SET email_verified_at = now()")
        .execute(&a.pool)
        .await
        .unwrap();
    let b = mail("<2@example.jp>");
    let (s, _, v) = App::send(
        &mail_app,
        Method::POST,
        "/api/two-way/tw/email",
        &signed(&secret, &b, now()),
        Some(b.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["forwarded"], true);
    assert!(
        v["message"]["translation"]
            .as_str()
            .unwrap()
            .contains("espace de travail")
    );
    let (to, subject, text): (String, String, String) = sqlx::query_as(
        "SELECT to_email, subject, text_body FROM email_outbox WHERE template = 'two_way_forward' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&a.pool)
    .await
    .unwrap();
    assert_eq!(to, "support@shop.example");
    assert_eq!(subject, "[ja → fr] Re: ご注文について");
    assert!(
        text.contains("espace de travail")
            && text.contains(JA)
            && !text.contains("votre colis est parti"),
        "{text}"
    );
    // The sender's address is not kept in glot's conversation tables.
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_conversations WHERE external_id LIKE '%@%' OR customer_name IS NOT NULL OR meta::text LIKE '%@%'").await,
        0
    );
    // Unsigned mail is refused.
    let (s, _, _) = App::send(
        &mail_app,
        Method::POST,
        "/api/two-way/tw/email",
        &[],
        Some(b),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

// ------------------------------------------------------------------------------------------------ Help Scout

#[derive(Default)]
struct FakeHelpScout {
    token_calls: usize,
    /// (method, path, body)
    log: Vec<(String, String, Value)>,
    threads: Value,
}

async fn fake_helpscout() -> (String, Arc<Mutex<FakeHelpScout>>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let shared: Arc<Mutex<FakeHelpScout>> = Default::default();
    let app = Router::new()
        .fallback(
            |State(s): State<Arc<Mutex<FakeHelpScout>>>, m: Method, u: Uri, h: HeaderMap, b: Bytes| async move {
                let mut f = s.lock().unwrap();
                if u.path() == "/v2/oauth2/token" {
                    let form = String::from_utf8_lossy(&b).into_owned();
                    if !(form.contains("grant_type=client_credentials")
                        && form.contains("client_id=app-1")
                        && form.contains("client_secret=app-secret"))
                    {
                        return (StatusCode::UNAUTHORIZED, axum::Json(json!({"error": "invalid_client"}))).into_response();
                    }
                    f.token_calls += 1;
                    return axum::Json(json!({"token_type": "bearer", "access_token": "tok-1", "expires_in": 172800}))
                        .into_response();
                }
                if h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some("Bearer tok-1") {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                let body: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
                f.log.push((m.to_string(), u.path().to_string(), body));
                match (m.as_str(), u.path()) {
                    ("GET", "/v2/conversations/7001/threads") => {
                        axum::Json(json!({"_embedded": {"threads": f.threads}})).into_response()
                    }
                    ("POST", "/v2/conversations/7001/notes") | ("POST", "/v2/conversations/7001/reply") => {
                        (StatusCode::CREATED, [("Resource-Id", "900")]).into_response()
                    }
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            },
        )
        .with_state(shared.clone());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, shared)
}

async fn wait_for(
    fake: &Arc<Mutex<FakeHelpScout>>,
    what: &str,
    pred: impl Fn(&FakeHelpScout) -> bool,
) {
    for _ in 0..200 {
        if pred(&fake.lock().unwrap()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "timed out waiting for: {what}; log = {:?}",
        fake.lock().unwrap().log
    );
}

#[tokio::test]
async fn helpscout_round_trip() {
    let (url, fake) = fake_helpscout().await;
    unsafe { std::env::set_var("GLOT_HELPSCOUT_API", &url) };
    let Some((a, _)) = setup().await else { return };
    let hook = "/api/two-way/tw/helpscout";
    let (s, v) = a
        .api(
            Method::PUT,
            "/two-way/settings",
            Some(json!({"helpscout_app_id": "app-1", "helpscout_app_secret": "app-secret", "helpscout_webhook_secret": "hs-hook"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v["helpscout"],
        json!({"app_id": "app-1", "has_app_secret": true, "has_webhook_secret": true})
    );
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT helpscout_app_secret_enc FROM two_way_settings")
            .fetch_one(&a.pool)
            .await
            .unwrap();
    assert!(
        !String::from_utf8_lossy(&stored).contains("app-secret"),
        "credentials are encrypted at rest"
    );

    let post = |event: &'static str, sig_secret: &'static str| {
        let body = json!({"id": 7001, "subject": "開けません", "primaryCustomer": {"id": 55, "email": "tanaka.yuki@example.jp"}})
            .to_string();
        let headers = vec![
            ("x-helpscout-event", event.to_string()),
            (
                "x-helpscout-signature",
                helpscout::sign(sig_secret, body.as_bytes()),
            ),
        ];
        let a = &a;
        async move { a.call(Method::POST, hook, &headers, Some(body)).await.0 }
    };
    let thread = |id: u64, kind: &str, body: &str| json!({"id": id, "type": kind, "body": body, "createdBy": {"id": 55, "type": "customer"}});

    // A forged webhook does nothing.
    assert_eq!(
        post("convo.created", "not-the-secret").await,
        StatusCode::UNAUTHORIZED
    );
    assert!(fake.lock().unwrap().log.is_empty());

    // 1. The customer writes in Japanese → a note with the French translation appears in Help Scout.
    fake.lock().unwrap().threads = json!([thread(1, "customer", &format!("<p>{JA}</p>"))]);
    assert_eq!(post("convo.created", "hs-hook").await, StatusCode::OK);
    wait_for(&fake, "translation note", |f| {
        f.log.iter().any(|(_, p, _)| p.ends_with("/notes"))
    })
    .await;
    {
        let f = fake.lock().unwrap();
        let note = f
            .log
            .iter()
            .find(|(_, p, _)| p.ends_with("/notes"))
            .unwrap()
            .2["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            note.contains("ja → fr")
                && note.contains("espace de travail")
                && note.contains("/glot"),
            "{note}"
        );
    }
    // Help Scout retries: no second note, no second message.
    assert_eq!(post("convo.created", "hs-hook").await, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        fake.lock()
            .unwrap()
            .log
            .iter()
            .filter(|(_, p, _)| p.ends_with("/notes"))
            .count(),
        1
    );
    assert_eq!(a.count("SELECT count(*) FROM two_way_messages").await, 1);
    // The customer's address in the payload was not kept.
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_conversations WHERE meta::text LIKE '%@%'")
            .await,
        0
    );

    // An ordinary internal note is none of glot's business.
    fake.lock().unwrap().threads = json!([
        thread(2, "note", "à rappeler demain"),
        thread(1, "customer", JA)
    ]);
    assert_eq!(post("convo.note.created", "hs-hook").await, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(a.count("SELECT count(*) FROM two_way_messages").await, 1);

    // 2. Suggest mode: `/glot …` becomes a DRAFT reply in Japanese; nothing is sent.
    let answer = "Bonjour, votre espace de travail est rouvert. La commande #48213 est en route.";
    fake.lock().unwrap().threads = json!([
        thread(3, "note", &format!("/glot {answer}")),
        thread(1, "customer", JA)
    ]);
    assert_eq!(post("convo.note.created", "hs-hook").await, StatusCode::OK);
    wait_for(&fake, "report note", |f| {
        f.log
            .iter()
            .filter(|(_, p, _)| p.ends_with("/notes"))
            .count()
            == 2
    })
    .await;
    {
        let f = fake.lock().unwrap();
        let replies: Vec<&Value> = f
            .log
            .iter()
            .filter(|(_, p, _)| p.ends_with("/reply"))
            .map(|l| &l.2)
            .collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["draft"], true);
        assert_eq!(replies[0]["customer"], json!({"id": 55}));
        let text = replies[0]["text"].as_str().unwrap();
        assert!(
            text.contains("ワークスペース") && text.contains("#48213"),
            "{text}"
        );
        let report = f
            .log
            .iter()
            .rev()
            .find(|(_, p, _)| p.ends_with("/notes"))
            .unwrap()
            .2["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            report.contains("Back-translation") && report.contains("draft"),
            "{report}"
        );
    }
    let (status, via, sent): (
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, delivered_via, sent_at FROM two_way_messages WHERE direction = 'out'",
    )
    .fetch_one(&a.pool)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), via.as_deref(), sent),
        ("draft", Some("helpscout_draft"), None)
    );

    // 3. Auto mode: the next `/glot` note is sent as a real reply.
    a.api(
        Method::PUT,
        "/two-way/settings",
        Some(json!({"mode": "auto", "review_below": 0.5})),
    )
    .await;
    fake.lock().unwrap().threads = json!([
        thread(4, "note", &format!("<p>/glot {answer}</p>")),
        thread(1, "customer", JA)
    ]);
    assert_eq!(post("convo.note.created", "hs-hook").await, StatusCode::OK);
    wait_for(&fake, "sent reply", |f| {
        f.log
            .iter()
            .filter(|(_, p, _)| p.ends_with("/reply"))
            .count()
            == 2
    })
    .await;
    wait_for(&fake, "second report", |f| {
        f.log
            .iter()
            .filter(|(_, p, _)| p.ends_with("/notes"))
            .count()
            == 3
    })
    .await;
    {
        let f = fake.lock().unwrap();
        let last = f
            .log
            .iter()
            .rev()
            .find(|(_, p, _)| p.ends_with("/reply"))
            .unwrap();
        assert_eq!(last.2["draft"], false);
        // One OAuth token for the whole exchange (Help Scout asks not to mint one per call).
        assert_eq!(f.token_calls, 1);
    }
    assert_eq!(
        a.count("SELECT count(*) FROM two_way_messages WHERE direction = 'out' AND status = 'sent' AND delivered_via = 'helpscout'").await,
        1
    );
    // The inbox shows the same thread.
    let (_, list) = a.api(Method::GET, "/two-way/conversations", None).await;
    assert_eq!(list["conversations"][0]["channel"], "helpscout");
    assert_eq!(list["conversations"][0]["messages"], 3);
}

// ------------------------------------------------------------------------------------------------ real data

/// Not a pass/fail test: runs the local parts of the pipeline (detection, masking, glossary pivot) on the
/// real ClipOnIt and Orizn catalogs and prints what they give. Read-only (SELECT) on the scrubbed copy `glot_real`.
/// `cargo test -p glot-server --test two_way real_data -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn real_data() {
    let url = std::env::var("GLOT_REAL_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot_real".into());
    let pool = PgPool::connect(&url).await.expect("glot_real database");
    let projects: Vec<(uuid::Uuid, String, String, Vec<String>)> = sqlx::query_as(
        "SELECT id, slug, source_locale, target_locales FROM projects ORDER BY slug",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (project, slug, source, targets) in projects {
        real_project(&pool, project, &slug, source, targets).await;
    }
}

async fn real_project(
    pool: &PgPool,
    project: uuid::Uuid,
    slug: &str,
    source: String,
    targets: Vec<String>,
) {
    let pool = pool.clone();
    // (locale, text): every source string and every translation.
    let mut texts: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.locale, t.value, k.source FROM translations t JOIN keys k ON k.id = t.key_id WHERE k.project_id = $1",
    )
    .bind(project)
    .fetch_all(&pool)
    .await
    .unwrap();
    let sources: Vec<(String,)> = sqlx::query_as("SELECT source FROM keys WHERE project_id = $1")
        .bind(project)
        .fetch_all(&pool)
        .await
        .unwrap();
    texts.extend(
        sources
            .into_iter()
            .map(|(s,)| (source.clone(), s.clone(), s)),
    );
    // Strings still carrying the offline mock provider's `⟦xx⟧ ` marker are not in the language they claim:
    // counted, then left out of the measurements.
    let marked = |t: &str| t.trim_start().starts_with('⟦');
    let marked_all = texts.iter().filter(|(_, t, _)| marked(t)).count();
    let marked_sources = texts
        .iter()
        .filter(|(l, t, _)| *l == source && marked(t))
        .count();
    texts.retain(|(_, t, _)| !marked(t));
    let glossary = Glossary {
        terms: sqlx::query_as::<_, (String, bool, bool, sqlx::types::Json<std::collections::BTreeMap<String, String>>)>(
            "SELECT term, do_not_translate, case_sensitive, translations FROM glossary_terms WHERE project_id = $1",
        )
        .bind(project)
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(term, dnt, cs, tr)| Term {
            term,
            description: None,
            do_not_translate: dnt,
            case_sensitive: cs,
            translations: tr.0,
            forbidden: Default::default(),
        })
        .collect(),
    };
    println!(
        "\n\n######## {slug}: {} real strings measured, {} glossary terms, source {source}, targets {targets:?}",
        texts.len(),
        glossary.terms.len()
    );
    println!(
        "left out: {marked_all} strings carrying the mock provider's ⟦xx⟧ marker ({marked_sources} of them source strings)"
    );

    // 1. Detection: accuracy and share above the 0.5 threshold, by length.
    println!("\n== Language detection (local), by text length");
    println!(
        "{:<14}{:>8}{:>12}{:>14}{:>20}",
        "letters", "strings", "confident", "right if conf.", "right (any conf.)"
    );
    for (name, lo, hi) in [
        ("1-9", 1, 9),
        ("10-19", 10, 19),
        ("20-39", 20, 39),
        ("40-79", 40, 79),
        ("80+", 80, usize::MAX),
    ] {
        let (mut n, mut conf, mut conf_ok, mut ok) = (0, 0, 0, 0);
        for (locale, text, _) in &texts {
            let letters = text.chars().filter(|c| c.is_alphabetic()).count();
            if letters < lo || letters > hi {
                continue;
            }
            let d = two_way::detect::detect(text);
            let right = d.locale.as_deref() == Some(two_way::lang(locale).as_str());
            n += 1;
            ok += right as usize;
            if d.confidence >= 0.5 {
                conf += 1;
                conf_ok += right as usize;
            }
        }
        let pct = |a: usize, b: usize| {
            if b == 0 {
                "-".to_string()
            } else {
                format!("{:.1}%", 100.0 * a as f64 / b as f64)
            }
        };
        println!(
            "{name:<14}{n:>8}{:>12}{:>14}{:>20}",
            pct(conf, n),
            pct(conf_ok, conf),
            pct(ok, n)
        );
    }

    // 2. Masking on text that contains no personal data: every hit is a false positive.
    let mut hits = std::collections::BTreeMap::<String, (usize, String)>::new();
    let mut masked_strings = 0;
    for (_, text, _) in &texts {
        let mut m = two_way::pii::Masker::default();
        let out = m.mask(text);
        if m.count() > 0 {
            masked_strings += 1;
            for kind in ["url", "email", "iban", "card", "phone", "ref"] {
                if out.contains(&format!("{{{kind}1}}")) {
                    let e = hits.entry(kind.to_string()).or_insert((0, text.clone()));
                    e.0 += 1;
                }
            }
        }
    }
    // What the confident-but-wrong guesses on 40+ letters are, by (expected → guessed).
    let mut wrong = std::collections::BTreeMap::<String, usize>::new();
    for (locale, text, _) in &texts {
        if text.chars().filter(|c| c.is_alphabetic()).count() < 40 {
            continue;
        }
        let d = two_way::detect::detect(text);
        let want = two_way::lang(locale);
        if d.confidence >= 0.5 && d.locale.as_deref() != Some(want.as_str()) {
            *wrong
                .entry(format!(
                    "{want} → {}",
                    d.locale.as_deref().unwrap_or("none")
                ))
                .or_default() += 1;
        }
    }
    println!("confident and wrong on 40+ letters: {wrong:?}");
    println!("\n== PII masking on product copy (no personal data expected)");
    println!(
        "{masked_strings} of {} strings had something hidden ({:.2}%)",
        texts.len(),
        100.0 * masked_strings as f64 / texts.len() as f64
    );
    for (kind, (n, example)) in &hits {
        println!(
            "  {kind:<6}{n:>5}  e.g. {:?}",
            example.chars().take(90).collect::<String>()
        );
    }

    // 3. Glossary pivot, checked against the real French source: a customer writing the Spanish (German,
    //    English) form of a term must get the French term the site uses.
    println!(
        "\n== Glossary pivot customer language → {source} (ground truth: the real source string)"
    );
    for from in targets.iter().map(String::as_str) {
        let (mut with_term, mut agrees) = (0, 0);
        let mut example = None;
        let mut miss = None;
        for (locale, text, src) in &texts {
            if locale != from {
                continue;
            }
            for t in two_way::pivot_glossary(&glossary, &source, from, &source, text) {
                if t.do_not_translate {
                    continue;
                }
                with_term += 1;
                let want = &t.translations[&source];
                if two_way::contains_term(&src.to_lowercase(), &want.to_lowercase()) {
                    agrees += 1;
                    example.get_or_insert_with(|| {
                        format!("{:?} → must use {:?}; real source: {:?}", text, want, src)
                    });
                } else {
                    miss.get_or_insert_with(|| {
                        format!(
                            "{:?} → would enforce {:?}; real source: {:?}",
                            text, want, src
                        )
                    });
                }
            }
        }
        println!(
            "{from} → {source}: {with_term} term occurrences in real {from} strings, {agrees} where the real {source} string uses the enforced term"
        );
        if let Some(e) = example {
            println!("   agrees: {e}");
        }
        if let Some(e) = miss {
            println!("   differs: {e}");
        }
    }
}
