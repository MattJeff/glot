//! Text graph on a small fixture project (real Postgres, skipped when unreachable): one identity per sentence
//! across catalog keys, a connector field and glot Edge segments; propagation rules; inconsistencies; importer.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

const G: &str = "/api/projects/demo/text-graph";

struct Ctx {
    app: axum::Router,
    pool: PgPool,
    cookie: String,
    project: uuid::Uuid,
}

impl Ctx {
    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header(header::COOKIE, format!("glot_session={}", self.cookie))
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn get(&self, path: &str) -> Value {
        let (s, v) = self.call(Method::GET, &format!("{G}{path}"), None).await;
        assert_eq!(s, StatusCode::OK, "GET {path}: {v}");
        v
    }
    async fn post(&self, path: &str, body: Value) -> Value {
        let (s, v) = self
            .call(Method::POST, &format!("{G}{path}"), Some(body))
            .await;
        assert_eq!(s, StatusCode::OK, "POST {path}: {v}");
        v
    }
    async fn key(&self, key: &str, source: &str) {
        let (s, v) = self
            .call(
                Method::POST,
                "/api/projects/demo/keys",
                Some(json!({ "key": key, "source": source })),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    /// A human translation through the normal API (lands `reviewed`).
    async fn reviewed(&self, key: &str, value: &str) {
        let (s, v) = self
            .call(
                Method::PUT,
                &format!("/api/projects/demo/keys/{key}/translations/fr"),
                Some(json!({ "value": value })),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    /// A machine translation, as a translation job stores it.
    async fn machine(&self, key: &str, value: &str) {
        sqlx::query(
            "INSERT INTO translations (key_id, locale, value, status, origin, source_hash)
             SELECT id, 'fr', $3, 'machine', 'machine', '' FROM keys WHERE project_id = $1 AND key = $2",
        )
        .bind(self.project)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .unwrap();
    }
    /// A segment glot Edge saw on a page; returns its locator in the graph.
    async fn segment(&self, text: &str, context: &str) -> String {
        let hash = glot_core::edge::tm::text_hash("en", text);
        sqlx::query(
            "INSERT INTO edge_segments (project_id, hash, context, source_text, url_sample, hits)
             VALUES ($1, $2, $3, $4, $5, 7)",
        )
        .bind(self.project)
        .bind(&hash)
        .bind(context)
        .bind(text)
        .bind(context.split('|').nth(1))
        .execute(&self.pool)
        .await
        .unwrap();
        format!("{hash}|{context}")
    }
    /// (value, status) of the catalog translation of `key` in fr.
    async fn fr(&self, key: &str) -> Option<(String, String)> {
        sqlx::query_as(
            "SELECT t.value, t.status FROM translations t JOIN keys k ON k.id = t.key_id
             WHERE k.project_id = $1 AND k.key = $2 AND t.locale = 'fr'",
        )
        .bind(self.project)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }
    /// The one sentence a search returns.
    async fn find(&self, q: &str) -> Value {
        let v = self
            .get(&format!("/search?q={}", q.replace(' ', "%20")))
            .await;
        let list = v["anchors"].as_array().unwrap();
        assert_eq!(list.len(), 1, "search {q}: {v}");
        list[0].clone()
    }
    async fn sync(&self) {
        self.post("/sync", json!({})).await;
    }
}

fn is(pair: Option<(String, String)>, value: &str, status: &str) {
    assert_eq!(pair, Some((value.to_string(), status.to_string())));
}

#[tokio::test]
async fn one_sentence_everywhere() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
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
    let app = glot_server::router(
        glot_server::AppState::new(pool.clone(), "test-secret", false),
        "http://localhost:3100",
    );
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"email": "t@x.io", "password": "correct horse", "name": "T"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let cookie = res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    let mut c = Ctx {
        app,
        pool: pool.clone(),
        cookie,
        project: uuid::Uuid::nil(),
    };
    let (s, _) = c
        .call(
            Method::POST,
            "/api/projects",
            Some(json!({"name": "D", "slug": "demo", "source_locale": "en", "target_locales": ["fr"]})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    c.project = sqlx::query_scalar("SELECT id FROM projects WHERE slug = 'demo'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // An empty project is an empty graph, not an error.
    let o = c.get("").await;
    assert_eq!(
        (o["anchors"].as_i64(), o["occurrences"].as_i64()),
        (Some(0), Some(0))
    );

    // ---- Dedup: the same sentence in two catalog keys, a Shopify field and a page glot Edge translated.
    let trial = "Start your free trial today";
    c.key("home.cta", trial).await;
    c.key("pricing.cta", trial).await;
    c.key("shopify:product:42:title", trial).await;
    let account = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO connector_accounts (id, project_id, kind, name, creds_enc, webhook_secret)
         VALUES ($1, $2, 'shopify', 'My shop', '', 'x')",
    )
    .bind(account)
    .bind(c.project)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO connector_items (account_id, external_id, field, key, format, source, source_hash)
         VALUES ($1, '42', 'title', 'shopify:product:42:title', 'plain', $2, '')",
    )
    .bind(account)
    .bind(trial)
    .execute(&pool)
    .await
    .unwrap();
    // Edge collapses whitespace before hashing: same sentence.
    let edge_trial = c.segment(trial, "h1|/pricing").await;
    c.sync().await;

    let a = c.find("free trial").await;
    let trial_id = a["id"].as_str().unwrap().to_string();
    assert_eq!(a["occurrences"], 4, "{a}");
    assert_eq!(a["surfaces"], json!(["catalog", "connector", "edge"]));
    // Found by the address of one of its places too (a key name).
    assert_eq!(c.find("pricing.cta").await["id"], trial_id.as_str());
    let o = c.get("").await;
    assert_eq!(
        (o["anchors"].as_i64(), o["cross_surface"].as_i64()),
        (Some(1), Some(1))
    );
    assert_eq!(
        o["surfaces"],
        json!({"catalog": 2, "connector": 1, "edge": 1})
    );

    // ---- Translate once, apply everywhere.
    c.reviewed("home.cta", "Commencez votre essai gratuit")
        .await;
    let d = c.get(&format!("/anchors/{trial_id}")).await;
    assert_eq!(d["locales"][0]["state"], "partial", "{d}");
    assert_eq!(d["locales"][0]["fillable"], 2);
    let shop = d["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["surface"] == "connector")
        .unwrap();
    assert_eq!(shop["meta"]["account"], "My shop");
    // The Edge segment has no translation of its own: Edge already serves the catalog one. Never duplicated.
    let seg = d["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["locator"] == edge_trial.as_str())
        .unwrap();
    assert_eq!(seg["translations"]["fr"]["inherited"], true, "{seg}");
    assert_eq!(seg["meta"]["hits"], 7);

    let dry = c.post("/propagate", json!({"dry_run": true})).await;
    assert_eq!(
        (dry["writes"].as_i64(), dry["written"].as_i64()),
        (Some(2), Some(0))
    );
    assert_eq!(c.fr("pricing.cta").await, None);
    let done = c.post("/propagate", json!({})).await;
    assert_eq!(done["written"], 2, "{done}");
    // A reviewed sentence stays reviewed where it is copied, and the catalog history says who wrote it.
    is(
        c.fr("pricing.cta").await,
        "Commencez votre essai gratuit",
        "reviewed",
    );
    is(
        c.fr("shopify:product:42:title").await,
        "Commencez votre essai gratuit",
        "reviewed",
    );
    let author: Option<String> = sqlx::query_scalar(
        "SELECT h.author FROM translation_history h JOIN keys k ON k.id = h.key_id WHERE k.key = 'pricing.cta'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(author.as_deref(), Some("text graph"));
    let edge_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM edge_translations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(edge_rows, 0);
    assert_eq!(c.post("/propagate", json!({})).await["written"], 0);
    assert_eq!(
        c.get(&format!("/anchors/{trial_id}")).await["locales"][0]["state"],
        "consistent"
    );

    // ---- Identity survives a key rename: same anchor, and the renamed key gets its translation back.
    let (s, _) = c
        .call(Method::DELETE, "/api/projects/demo/keys/home.cta", None)
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    c.key("landing.hero.cta", trial).await;
    c.sync().await;
    let a = c.find("free trial").await;
    assert_eq!(a["id"], trial_id.as_str());
    assert_eq!(a["occurrences"], 4);
    let d = c.get(&format!("/anchors/{trial_id}")).await;
    let locators: Vec<&str> = d["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["locator"].as_str().unwrap())
        .collect();
    assert!(
        locators.contains(&"landing.hero.cta") && !locators.contains(&"home.cta"),
        "{locators:?}"
    );
    assert_eq!(c.post("/propagate", json!({})).await["written"], 1);
    is(
        c.fr("landing.hero.cta").await,
        "Commencez votre essai gratuit",
        "reviewed",
    );

    // ---- Edge <-> catalog: a translation reviewed on the live page reaches the catalog key, and replaces the
    // machine translation of the same sentence elsewhere on the site.
    let ship = "Free shipping on all orders";
    c.key("banner.shipping", ship).await;
    let ship_p = c.segment(ship, "p|/").await;
    let ship_h2 = c.segment(ship, "h2|/cart").await;
    for (locator, value, status, origin) in [
        (
            &ship_p,
            "Livraison gratuite sur toutes les commandes",
            "reviewed",
            "human",
        ),
        (
            &ship_h2,
            "Envoi gratuit pour toutes les commandes",
            "machine",
            "machine",
        ),
    ] {
        let (hash, context) = locator.split_once('|').unwrap();
        sqlx::query(
            "INSERT INTO edge_translations (project_id, hash, context, locale, value, status, origin)
             VALUES ($1, $2, $3, 'fr', $4, $5, $6)",
        )
        .bind(c.project)
        .bind(hash)
        .bind(context)
        .bind(value)
        .bind(status)
        .bind(origin)
        .execute(&pool)
        .await
        .unwrap();
    }
    let done = c.post("/propagate", json!({})).await;
    assert_eq!(done["written"], 2, "{done}");
    is(
        c.fr("banner.shipping").await,
        "Livraison gratuite sur toutes les commandes",
        "reviewed",
    );
    let (value, status): (String, String) =
        sqlx::query_as("SELECT value, status FROM edge_translations WHERE context = 'h2|/cart'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (value.as_str(), status.as_str()),
        ("Livraison gratuite sur toutes les commandes", "reviewed")
    );

    // ---- Inconsistency: two machine translations of one sentence. Detected, never settled automatically.
    let del = "Delete your account forever";
    for k in ["settings.delete", "modal.delete", "help.delete"] {
        c.key(k, del).await;
    }
    c.machine("settings.delete", "Supprimer votre compte définitivement")
        .await;
    c.machine("modal.delete", "Effacer votre compte pour toujours")
        .await;
    // ---- Conflict: two people reviewed the same sentence differently.
    let hi = "Welcome back to your dashboard";
    c.key("a.welcome", hi).await;
    c.key("b.welcome", hi).await;
    c.reviewed("a.welcome", "Bon retour sur votre tableau de bord")
        .await;
    c.reviewed(
        "b.welcome",
        "Content de vous revoir sur votre tableau de bord",
    )
    .await;
    c.sync().await;

    let o = c.get("").await;
    assert_eq!(
        (o["inconsistent"].as_i64(), o["conflicts"].as_i64()),
        (Some(1), Some(1)),
        "{o}"
    );
    let bad = o["inconsistencies"].as_array().unwrap();
    let inc = bad.iter().find(|b| b["state"] == "inconsistent").unwrap();
    assert_eq!(inc["text"], del);
    assert_eq!(inc["values"].as_array().unwrap().len(), 2);
    let del_id = inc["anchor_id"].as_str().unwrap().to_string();
    let hi_id = bad.iter().find(|b| b["state"] == "conflict").unwrap()["anchor_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Automatic propagation guesses nothing: help.delete stays empty, both reviewed translations stay.
    assert_eq!(c.post("/propagate", json!({})).await["written"], 0);
    assert_eq!(c.fr("help.delete").await, None);
    for id in [&del_id, &hi_id] {
        let (s, v) = c
            .call(
                Method::POST,
                &format!("{G}/anchors/{id}/unify"),
                Some(json!({"locale": "fr"})),
            )
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "{v}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains("disagree"),
            "{v}"
        );
    }

    // One action unifies the machine translations: the chosen one lands everywhere, reviewed.
    let u = c
        .post(
            &format!("/anchors/{del_id}/unify"),
            json!({"locale": "fr", "value": "Supprimer votre compte définitivement"}),
        )
        .await;
    assert_eq!(u["written"], 2, "{u}");
    for k in ["modal.delete", "help.delete"] {
        is(
            c.fr(k).await,
            "Supprimer votre compte définitivement",
            "reviewed",
        );
    }

    // A reviewed translation is not replaced unless a person says so explicitly: the conflict is reported.
    let keep = "Bon retour sur votre tableau de bord";
    let u = c
        .post(
            &format!("/anchors/{hi_id}/unify"),
            json!({"locale": "fr", "value": keep}),
        )
        .await;
    assert_eq!(
        (u["written"].as_i64(), u["skipped"].as_i64()),
        (Some(0), Some(1)),
        "{u}"
    );
    assert_eq!(u["actions"][0]["reason"], "reviewed_conflict");
    assert_eq!(u["actions"][0]["locator"], "b.welcome");
    is(
        c.fr("b.welcome").await,
        "Content de vous revoir sur votre tableau de bord",
        "reviewed",
    );
    let u = c
        .post(
            &format!("/anchors/{hi_id}/unify"),
            json!({"locale": "fr", "value": keep, "force": true}),
        )
        .await;
    assert_eq!(u["written"], 1);
    is(c.fr("b.welcome").await, keep, "reviewed");
    let o = c.get("").await;
    assert_eq!(
        (o["inconsistent"].as_i64(), o["conflicts"].as_i64()),
        (Some(0), Some(0))
    );

    // A machine translation never replaces a reviewed one, whatever the majority says.
    let pay = "Your payment method was declined";
    for k in ["pay.a", "pay.b", "pay.c"] {
        c.key(k, pay).await;
    }
    c.machine("pay.a", "Votre moyen de paiement a été rejeté")
        .await;
    c.machine("pay.b", "Votre moyen de paiement a été rejeté")
        .await;
    c.reviewed("pay.c", "Votre moyen de paiement a été refusé")
        .await;
    assert_eq!(c.post("/propagate", json!({})).await["written"], 2);
    for k in ["pay.a", "pay.b", "pay.c"] {
        is(
            c.fr(k).await,
            "Votre moyen de paiement a été refusé",
            "reviewed",
        );
    }

    // ---- Context keeps sentences distinct: "Save" on a button is not "Save" in a price table.
    c.key("form.save", "Save").await;
    sqlx::query("UPDATE keys SET ui_role = 'button' WHERE key = 'form.save'")
        .execute(&pool)
        .await
        .unwrap();
    c.key("promo.save", "Save").await;
    c.segment("Save", "td|/pricing").await;
    c.reviewed("form.save", "Enregistrer").await;
    c.sync().await;
    let v = c.get("/search?q=form.save").await;
    let verb = v["anchors"][0].clone();
    let v = c.get("/search?q=promo.save").await;
    let noun = v["anchors"][0].clone();
    assert_ne!(verb["id"], noun["id"]);
    assert_eq!(
        (verb["context"].as_str(), noun["context"].as_str()),
        (Some("action"), Some(""))
    );
    assert_eq!(noun["occurrences"], 2, "{noun}");
    assert_eq!(c.post("/propagate", json!({})).await["written"], 0);
    assert_eq!(c.fr("promo.save").await, None);
    // Not hidden: each sentence names the other as the same text in another context.
    let d = c
        .get(&format!("/anchors/{}", noun["id"].as_str().unwrap()))
        .await;
    assert_eq!(d["related"][0]["id"], verb["id"], "{d}");
    assert_eq!(d["related"][0]["kind"], "context");

    // ---- Importer contract: an email template plugs in, with printf placeholders where the app uses ICU.
    c.key("mail.greeting", "Hello {name}, welcome back").await;
    c.reviewed("mail.greeting", "Bonjour {name}, bon retour")
        .await;
    let i = c
        .post(
            "/import",
            json!({"surface": "email", "replace": true, "items": [
                {"locator": "welcome/preheader", "text": "Hello %s, welcome back", "url": "https://mail.example/welcome"},
                {"locator": "welcome/cta", "text": trial, "role": "button"},
                {"locator": "welcome/footer", "text": "You receive this email because you signed up."},
            ]}),
        )
        .await;
    assert_eq!(i["imported"], 3, "{i}");
    assert_eq!(i["items"][0]["elsewhere"], 1);
    assert_eq!(i["items"][1]["anchor_id"], trial_id.as_str());
    assert_eq!(i["items"][2]["elsewhere"], 0);
    // Known at once, in the placeholder syntax of the email, before anything is written.
    let e = c.get("/export?surface=email&locale=fr").await;
    assert_eq!(
        e["strings"]["welcome/preheader"]["value"], "Bonjour %s, bon retour",
        "{e}"
    );
    assert_eq!(e["strings"]["welcome/preheader"]["from"], "graph");
    assert_eq!(
        e["strings"]["welcome/cta"]["value"],
        "Commencez votre essai gratuit"
    );
    assert_eq!(e["missing"], json!(["welcome/footer"]));
    assert_eq!(c.post("/propagate", json!({})).await["written"], 2);
    let e = c.get("/export?surface=email&locale=fr").await;
    assert_eq!(e["strings"]["welcome/preheader"]["from"], "own");
    assert_eq!(e["strings"]["welcome/preheader"]["status"], "reviewed");
    assert_eq!(
        c.find("free trial").await["surfaces"],
        json!(["catalog", "connector", "edge", "email"])
    );
    // replace=true: the payload is the whole surface.
    let i = c
        .post(
            "/import",
            json!({"surface": "email", "replace": true, "items": [{"locator": "welcome/cta", "text": trial}]}),
        )
        .await;
    assert_eq!(i["removed"], 2);
    // Every refusal says what to send instead.
    for (body, needle) in [
        (json!({"surface": "catalog", "items": []}), "own store"),
        (
            json!({"surface": "email", "items": [{"locator": "x", "text": " "}]}),
            "items[0]",
        ),
        (
            json!({"surface": "email", "items": [{"locator": "x", "text": "y", "translations": {"pt": "z"}}]}),
            "not a target locale",
        ),
    ] {
        let (s, v) = c
            .call(Method::POST, &format!("{G}/import"), Some(body))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert!(
            v["error"]["message"].as_str().unwrap().contains(needle),
            "{v}"
        );
    }

    // ---- Change the source once: the places still showing the old wording are known.
    let (s, _) = c
        .call(
            Method::PATCH,
            "/api/projects/demo/keys/pricing.cta",
            Some(json!({"source": "Start your free trial now"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    c.sync().await;
    let o = c.get("").await;
    let r = &o["reworded"][0];
    assert_eq!(r["from"]["id"], trial_id.as_str(), "{o}");
    assert_eq!(r["to"]["text"], "Start your free trial now");
    assert_eq!(
        (r["moved"].as_i64(), r["remaining"].as_i64()),
        (Some(1), Some(4))
    );
    let d = c.get(&format!("/anchors/{trial_id}")).await;
    assert_eq!(d["moved"][0]["locator"], "pricing.cta");

    // Another member's project is not readable.
    let (s, _) = c
        .call(Method::GET, "/api/projects/other/text-graph", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// Field run on a COPY of a real project database (it is migrated to this code): prints what the graph finds.
/// `GLOT_TEXT_GRAPH_DB=postgres://…/a_copy GLOT_TEXT_GRAPH_SLUG=cliponit cargo test -p glot-server --test
/// text_graph -- --ignored field_report --nocapture`. Translations are only read (propagation runs as a dry run).
#[tokio::test]
#[ignore]
async fn field_report() {
    let url = std::env::var("GLOT_TEXT_GRAPH_DB").expect("GLOT_TEXT_GRAPH_DB");
    let slug = std::env::var("GLOT_TEXT_GRAPH_SLUG").expect("GLOT_TEXT_GRAPH_SLUG");
    let pool = PgPool::connect(&url).await.unwrap();
    // A copy of a production database is usually a few migrations behind this code.
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let project: uuid::Uuid = sqlx::query_scalar("SELECT id FROM projects WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&pool)
        .await
        .unwrap();
    let token = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, expires_at)
         SELECT $1, user_id, now() + interval '1 hour' FROM project_members
         WHERE project_id = $2 AND role = 'owner' LIMIT 1",
    )
    .bind(glot_server::auth::sha256_hex(&token))
    .bind(project)
    .execute(&pool)
    .await
    .unwrap();
    let c = Ctx {
        app: glot_server::router(
            glot_server::AppState::new(pool.clone(), "field", false),
            "http://localhost:3100",
        ),
        pool: pool.clone(),
        cookie: token,
        project,
    };
    let base = format!("/api/projects/{slug}/text-graph");
    let started = std::time::Instant::now();
    let (s, o) = c.call(Method::GET, &base, None).await;
    assert_eq!(s, StatusCode::OK, "{o}");
    println!("first overview (sync included): {:?}", started.elapsed());
    let mut brief = o.clone();
    brief["inconsistencies"] = json!(o["inconsistencies"].as_array().map(Vec::len));
    println!("OVERVIEW {brief:#}");
    for i in o["inconsistencies"].as_array().unwrap().iter().take(12) {
        let values: Vec<String> = i["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("{:?} x{}", v["value"].as_str().unwrap(), v["count"]))
            .collect();
        println!(
            "TWO WAYS [{}] {:?}: {}",
            i["locale"].as_str().unwrap(),
            i["text"].as_str().unwrap(),
            values.join(" | ")
        );
    }
    let (_, top) = c
        .call(Method::GET, &format!("{base}/search?limit=8"), None)
        .await;
    for a in top["anchors"].as_array().unwrap() {
        println!(
            "TOP {} places {} {:?}",
            a["occurrences"],
            a["surfaces"],
            a["text"].as_str().unwrap()
        );
    }
    let (s, dry) = c
        .call(
            Method::POST,
            &format!("{base}/propagate"),
            Some(json!({"dry_run": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{dry}");
    println!(
        "PROPAGATE dry run: writes {} skipped {} unsettled {}",
        dry["writes"], dry["skipped"], dry["unsettled"]
    );
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for a in dry["actions"].as_array().unwrap() {
        *by.entry(format!(
            "{} {} {}",
            a["action"].as_str().unwrap(),
            a["surface"].as_str().unwrap(),
            a["reason"].as_str().unwrap_or("-")
        ))
        .or_default() += 1;
    }
    println!("first 500 actions by kind: {by:?}");
    for a in dry["actions"].as_array().unwrap().iter().take(6) {
        println!("ACTION {a}");
    }
}
