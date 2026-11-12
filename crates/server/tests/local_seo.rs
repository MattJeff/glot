//! Local SEO end to end (real Postgres, fake LLM provider, skipped when Postgres is unreachable): a market without
//! search data proposes nothing, regions of one language get distinct proposals, nothing is served before a human
//! approves (default mode), validators block approval, edits are re-validated, a changed source page takes an
//! override down, `auto` mode serves only clean proposals — and the Search Console adapter speaks the real API
//! shape (against a local fake of Google: no network).
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{Method, Request, StatusCode, header},
    routing::post,
};
use glot_server::local_seo::{
    self,
    demand::{DemandSource, Google, Query, SearchConsole},
    validate::market,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const HOME_HTML: &str = r#"<html><head><title>Recadrez vos vidéos | ClipOnIt</title>
<meta name="description" content="Recadrez vos vidéos en un clic avec ClipOnIt, pour tous vos réseaux."></head>
<body><h1>Recadrez vos vidéos</h1></body></html>"#;

fn home(title: &str) -> Value {
    json!({
        "path": "/", "title": title,
        "description": "Recadrez vos vidéos en un clic avec ClipOnIt, pour tous vos réseaux.",
        "h1": "Recadrez vos vidéos", "h2": [], "text": "Importez une vidéo, choisissez un format, exportez.",
    })
}

fn pricing() -> Value {
    json!({"path": "/pricing", "title": "Tarifs | ClipOnIt", "description": "", "h1": "Nos tarifs", "h2": [], "text": "Un abonnement mensuel."})
}

#[tokio::test]
async fn local_seo() {
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
    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                headers,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };

    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "seo@x.io", "password": "correct horse", "name": "S"})),
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
    let c = Some(cookie.as_str());
    let (s, _, b) = call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "ClipOnIt", "slug": "clip", "source_locale": "fr", "target_locales": ["es-ES", "es-MX", "de"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let project: uuid::Uuid = b["id"].as_str().unwrap().parse().unwrap();
    let (s, _, b) = call(
        Method::PATCH,
        "/api/projects/clip",
        c,
        Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _, _) = call(
        Method::POST,
        "/api/projects/clip/glossary",
        c,
        Some(json!({"term": "ClipOnIt", "do_not_translate": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let api = "/api/projects/clip/local-seo";
    let view = || async { call(Method::GET, api, c, None).await.2 };
    let row = |v: &Value, path: &str, locale: &str| -> Value {
        v["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["path"] == path && r["locale"] == locale)
            .cloned()
            .unwrap_or(Value::Null)
    };
    let live = |locale: &'static str| async move {
        call(
            Method::GET,
            &format!("/cdn/clip/local-seo/{locale}.json"),
            None,
            None,
        )
        .await
        .2["pages"]
            .clone()
    };

    // ----- Honest empty states: nothing scanned, Search Console neither configured nor connected.
    let v = view().await;
    assert_eq!(v["mode"], "review");
    assert_eq!(v["rows"], json!([]));
    assert_eq!(v["gsc"]["connected"], false);
    assert_eq!(v["markets"][1]["country"], "MX");
    let (s, _, b) = call(Method::POST, &format!("{api}/scan"), c, Some(json!({}))).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("no_site")),
        "{b}"
    );
    let (s, _, b) = call(Method::POST, &format!("{api}/metrics/refresh"), c, None).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("gsc_not_connected"))
    );
    if std::env::var("GOOGLE_OAUTH_CLIENT_ID").is_err() {
        let (s, _, b) = call(Method::GET, &format!("{api}/gsc/connect"), c, None).await;
        assert_eq!(
            (s, b["error"]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("gsc_not_configured"))
        );
    }
    let (s, _, _) = call(Method::POST, &format!("{api}/scan"), None, Some(json!({}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // ----- No search data: every market says so, no keyword and no proposal is made up.
    let pages = json!({"pages": [home("Recadrez vos vidéos | ClipOnIt"), pricing()]});
    let (s, _, b) = call(Method::POST, &format!("{api}/scan"), c, Some(pages.clone())).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["results"].as_array().unwrap().len(), 6);
    assert!(
        b["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "needs_demand"
                && r["note"].as_str().unwrap().contains("No search data")),
        "{b}"
    );
    let v = view().await;
    let r = row(&v, "/", "es-MX");
    assert_eq!(
        (r["terms"].clone(), r["proposed"]["title"].clone()),
        (json!([]), Value::Null)
    );
    assert!(r["audit"].is_array());
    let (s, _, b) = call(
        Method::POST,
        &format!("{api}/overrides/{}/approve", r["id"].as_str().unwrap()),
        c,
        None,
    )
    .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("nothing_to_approve"))
    );

    // ----- Real demand, per market: the two Spanish regions do not search the same words.
    for (locale, path, terms) in [
        (
            "es-ES",
            Some("/"),
            json!([{"term": "recortar vídeo online", "impressions": 5400}]),
        ),
        (
            "es-MX",
            Some("/"),
            json!([{"term": "cortar video en línea", "impressions": 8100}]),
        ),
        (
            "es-MX",
            None,
            json!([{"term": "editor de video gratis", "impressions": 900}]),
        ),
    ] {
        let (s, _, b) = call(
            Method::PUT,
            &format!("{api}/demand"),
            c,
            Some(json!({"locale": locale, "path": path, "terms": terms})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    let (s, _, b) = call(
        Method::PUT,
        &format!("{api}/demand"),
        c,
        Some(json!({"locale": "it", "terms": []})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let (s, _, b) = call(Method::POST, &format!("{api}/scan"), c, Some(pages.clone())).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v = view().await;
    let (es, mx) = (row(&v, "/", "es-ES"), row(&v, "/", "es-MX"));
    assert_eq!(es["status"], "proposed");
    assert_eq!(es["proposed"]["title"], "Recortar vídeo online | ClipOnIt");
    assert_eq!(mx["proposed"]["title"], "Cortar video en línea | ClipOnIt");
    assert_eq!(mx["proposed"]["slug"], "cortar-video-en-linea");
    assert_eq!(
        mx["terms"][0],
        json!({"term": "cortar video en línea", "source": "manual", "impressions": 8100, "clicks": null, "position": null})
    );
    assert_eq!(
        es["issues"],
        json!([]),
        "a clean proposal: only consent is missing"
    );
    assert_eq!(row(&v, "/", "de")["status"], "needs_demand");
    assert_eq!(row(&v, "/pricing", "es-ES")["status"], "needs_demand");
    // "gratis" is searched in Mexico, but the pricing page never says the product is free: validators say so.
    let mx_pricing = row(&v, "/pricing", "es-MX");
    assert_eq!(
        mx_pricing["proposed"]["title"],
        "Editor de video gratis | ClipOnIt"
    );
    assert_eq!(mx_pricing["issues"][0]["code"], "unsupported_claim");

    // ----- Default mode: nothing is applied without consent.
    assert_eq!(live("es-MX").await, json!({}));
    assert_eq!(live("es-ES").await, json!({}));
    assert_eq!(
        local_seo::rewrite(&pool, project, "es-MX", "/", HOME_HTML.into()).await,
        HOME_HTML
    );
    assert_eq!(local_seo::lookup(&pool, project, "es-MX", "/").await, None);

    // ----- Approval. Anonymous callers cannot; errors block; an approved override is served to its market only.
    let id = |r: &Value| r["id"].as_str().unwrap().to_string();
    let (s, _, _) = call(
        Method::POST,
        &format!("{api}/overrides/{}/approve", id(&mx)),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, b) = call(
        Method::POST,
        &format!("{api}/overrides/{}/approve", id(&mx_pricing)),
        c,
        None,
    )
    .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("has_errors")),
        "{b}"
    );
    assert!(
        b["error"]["message"].as_str().unwrap().contains("gratis"),
        "{b}"
    );
    let (s, _, b) = call(
        Method::POST,
        &format!("{api}/overrides/{}/approve", id(&mx)),
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        live("es-MX").await["/"]["title"],
        "Cortar video en línea | ClipOnIt"
    );
    assert_eq!(
        live("es-MX").await["/"]["slug"],
        Value::Null,
        "the slug stays a suggestion"
    );
    assert_eq!(
        live("es-ES").await,
        json!({}),
        "a sibling region is never used"
    );
    let out = local_seo::rewrite(&pool, project, "es-MX", "/?utm=1", HOME_HTML.into()).await;
    assert!(
        out.contains("<title>Cortar video en línea | ClipOnIt</title>")
            && out.contains("<h1>Cortar video en línea</h1>"),
        "{out}"
    );
    assert_eq!(
        local_seo::rewrite(&pool, project, "es-ES", "/", HOME_HTML.into()).await,
        HOME_HTML
    );
    assert!(row(&view().await, "/", "es-MX")["applied_at"].is_string());

    // ----- A human edit goes through the same validators (uniqueness across pages), then can be approved.
    let edit = |id: String, title: &'static str, approve: bool| async move {
        call(
            Method::PATCH,
            &format!("{api}/overrides/{id}"),
            c,
            Some(json!({"content": {"title": title}, "approve": approve})),
        )
        .await
    };
    let (s, _, b) = edit(id(&mx_pricing), "Cortar video en línea | ClipOnIt", true).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(
        b["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already uses this title"),
        "{b}"
    );
    assert_eq!(live("es-MX").await.get("/pricing"), None);
    let (s, _, b) = edit(
        id(&mx_pricing),
        "Precios para cortar video | ClipOnIt",
        false,
    )
    .await;
    assert_eq!(
        (s, b["status"].as_str(), b["issues"].clone()),
        (StatusCode::OK, Some("proposed"), json!([])),
        "{b}"
    );
    assert_eq!(
        live("es-MX").await.get("/pricing"),
        None,
        "an edit is still a draft"
    );
    let (s, _, _) = edit(
        id(&mx_pricing),
        "Precios para cortar video | ClipOnIt",
        true,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        live("es-MX").await["/pricing"],
        json!({"title": "Precios para cortar video | ClipOnIt", "description": null, "h1": null, "h2": [], "slug": null, "faq": []})
    );
    // Editing a live override takes it back to draft until approved again.
    let (s, _, _) = edit(
        id(&mx_pricing),
        "Precios y planes para cortar video | ClipOnIt",
        false,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(live("es-MX").await.get("/pricing"), None);

    // ----- Reject takes an override down.
    let (s, _, _) = call(
        Method::POST,
        &format!("{api}/overrides/{}/reject", id(&mx)),
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(live("es-MX").await, json!({}));
    // A rescan keeps the human's "no" on an unchanged page.
    call(Method::POST, &format!("{api}/scan"), c, Some(pages.clone())).await;
    assert_eq!(row(&view().await, "/", "es-MX")["status"], "rejected");

    // ----- The source page changes under a live override: it is no longer served until someone approves again.
    let (s, _, _) = call(
        Method::POST,
        &format!("{api}/overrides/{}/approve", id(&es)),
        c,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(live("es-ES").await["/"]["h1"], "Recortar vídeo online");
    let changed = json!({"pages": [home("Montez vos vidéos | ClipOnIt")]});
    let (s, _, b) = call(
        Method::POST,
        &format!("{api}/scan"),
        c,
        Some(changed.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(live("es-ES").await, json!({}));
    assert_eq!(row(&view().await, "/", "es-ES")["status"], "proposed");

    // ----- Auto mode is an explicit admin choice; it serves clean proposals and still holds back the others.
    let (s, _, _) = call(
        Method::PUT,
        &format!("{api}/settings"),
        c,
        Some(json!({"mode": "yolo"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, b) = call(
        Method::PUT,
        &format!("{api}/settings"),
        c,
        Some(json!({"mode": "auto"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let all =
        json!({"pages": [home("Montez vos vidéos | ClipOnIt"), pricing()], "regenerate": true});
    let (s, _, b) = call(Method::POST, &format!("{api}/scan"), c, Some(all)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v = view().await;
    assert_eq!(row(&v, "/", "es-ES")["status"], "auto");
    assert_eq!(
        live("es-ES").await["/"]["title"],
        "Recortar vídeo online | ClipOnIt"
    );
    assert_eq!(
        row(&v, "/pricing", "es-MX")["status"],
        "proposed",
        "errors are never auto-applied"
    );
    assert_eq!(live("es-MX").await.get("/pricing"), None);
    assert_eq!(row(&v, "/", "de")["status"], "needs_demand");
    // No Search Console rows stored: no number is shown.
    assert_eq!(
        row(&v, "/", "es-ES")["metrics"],
        json!({"before": null, "after": null})
    );

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}

// ---------- Search Console adapter against a local fake of Google ----------

#[derive(Clone, Default)]
struct Fake {
    queries: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn fake_query(
    State(f): State<Fake>,
    Path(site): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let filters = body["dimensionFilterGroups"][0]["filters"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let by_page = filters.iter().any(|x| x["dimension"] == "page");
    let rows = if body["dimensions"][0] == "date" {
        json!([{"keys": ["2026-11-01"], "impressions": 120, "clicks": 9, "ctr": 0.075, "position": 7.42}])
    } else if by_page {
        json!([{"keys": ["cortar video en línea"], "impressions": 300, "clicks": 12, "ctr": 0.04, "position": 8.26}])
    } else {
        json!([{"keys": ["editor de video"], "impressions": 900, "clicks": 4, "ctr": 0.004, "position": 21.0}])
    };
    f.queries.lock().unwrap().push((site, body));
    Json(json!({ "rows": rows }))
}

#[tokio::test]
async fn search_console_adapter_speaks_the_real_api() {
    let fake = Fake::default();
    let router = Router::new()
        .route(
            "/token",
            post(|body: String| async move {
                assert!(
                    body.contains("grant_type=refresh_token")
                        && body.contains("refresh_token=rt-1")
                        && body.contains("client_id=cid"),
                    "{body}"
                );
                Json(json!({"access_token": "at-1", "expires_in": 3600}))
            }),
        )
        .route(
            "/webmasters/v3/sites/{site}/searchAnalytics/query",
            post(fake_query),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let gsc = SearchConsole {
        google: Google {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            auth_url: format!("{base}/auth"),
            token_url: format!("{base}/token"),
            api_base: base.clone(),
        },
        site_url: "sc-domain:cliponit.app".into(),
        refresh_token: "rt-1".into(),
    };
    let consent = gsc
        .google
        .consent_url("https://glot.example/api/local-seo/gsc/callback", "st");
    assert!(
        consent.contains("webmasters.readonly")
            && consent.contains("access_type=offline")
            && consent.contains("state=st"),
        "{consent}"
    );

    // Demand of one page in Mexico: the page's own queries, then (fewer than five) the site's in that country.
    let mx = market("es-MX");
    let terms = gsc
        .terms(&Query {
            market: &mx,
            path: "/",
            page_url: Some("https://es.cliponit.app/"),
        })
        .await
        .unwrap();
    assert_eq!(terms.len(), 2);
    assert_eq!(
        (
            terms[0].term.as_str(),
            terms[0].source.as_str(),
            terms[0].impressions,
            terms[0].position
        ),
        ("cortar video en línea", "gsc", Some(300), Some(8.3))
    );
    assert_eq!(
        (terms[1].term.as_str(), terms[1].source.as_str()),
        ("editor de video", "gsc_site")
    );
    {
        let q = fake.queries.lock().unwrap();
        assert_eq!(q[0].0, "sc-domain:cliponit.app");
        assert_eq!(q[0].1["dimensions"], json!(["query"]));
        assert_eq!(
            q[0].1["dimensionFilterGroups"][0]["filters"],
            json!([
                {"dimension": "country", "operator": "equals", "expression": "mex"},
                {"dimension": "page", "operator": "equals", "expression": "https://es.cliponit.app/"},
            ])
        );
        assert_eq!(
            q[1].1["dimensionFilterGroups"][0]["filters"],
            json!([{"dimension": "country", "operator": "equals", "expression": "mex"}])
        );
    }
    // A bare language has no country: only the page's own queries, never the whole site's.
    fake.queries.lock().unwrap().clear();
    let es = market("es");
    gsc.terms(&Query {
        market: &es,
        path: "/",
        page_url: Some("https://es.cliponit.app/"),
    })
    .await
    .unwrap();
    assert_eq!(fake.queries.lock().unwrap().len(), 1);

    let days = gsc
        .daily(&mx, "https://es.cliponit.app/", 90)
        .await
        .unwrap();
    assert_eq!(days, vec![("2026-11-01".parse().unwrap(), 120, 9, 7.42)]);
}

// ---------- field run on a real project (not part of the suite) ----------

/// `LOCAL_SEO_FIELD_DB=postgres://…/<your own copy> cargo test -p glot-server --test local_seo field_run -- --ignored --nocapture`
/// Scans real public pages of the project `LOCAL_SEO_FIELD_SLUG` (default `cliponit`) in a **copy** of a real
/// database and prints what the dashboard would show. Public GETs only; no LLM call can happen unless the copy
/// holds real demand terms and an API key.
#[tokio::test]
#[ignore]
async fn field_run() {
    let Ok(url) = std::env::var("LOCAL_SEO_FIELD_DB") else {
        eprintln!("SKIPPED: set LOCAL_SEO_FIELD_DB to your own copy of a real database");
        return;
    };
    let slug = std::env::var("LOCAL_SEO_FIELD_SLUG").unwrap_or_else(|_| "cliponit".into());
    let paths: Vec<String> = std::env::var("LOCAL_SEO_FIELD_PATHS")
        .unwrap_or_else(|_| "/,/pricing".into())
        .split(',')
        .map(str::to_string)
        .collect();
    let pool = PgPool::connect(&url).await.unwrap();
    glot_server::MIGRATOR.run(&pool).await.unwrap();
    let app = glot_server::router(
        glot_server::AppState::new(pool.clone(), "field-secret", false),
        "http://localhost:3100",
    );
    let call = |method: Method, uri: String, cookie: Option<String>, body: Option<Value>| {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                headers,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let email = format!("field-{}@x.io", uuid::Uuid::new_v4().simple());
    let (s, h, b) = call(
        Method::POST,
        "/api/auth/signup".into(),
        None,
        Some(json!({"email": email, "password": "correct horse", "name": "Field"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()["glot_session=".len()..]
        .to_string();
    sqlx::query("INSERT INTO project_members (project_id, user_id, role) SELECT p.id, u.id, 'owner' FROM projects p, users u WHERE p.slug = $1 AND u.email = $2")
        .bind(&slug)
        .bind(&email)
        .execute(&pool)
        .await
        .unwrap();
    let api = format!("/api/projects/{slug}/local-seo");
    // As the project is: what the dashboard says before anyone configured anything.
    let (s, _, b) = call(
        Method::POST,
        format!("{api}/scan"),
        Some(cookie.clone()),
        Some(json!({"paths": []})),
    )
    .await;
    println!("SCAN-AS-IS {s} {b}");
    if let Ok(site) = std::env::var("LOCAL_SEO_FIELD_SITE") {
        let (s, _, b) = call(
            Method::PUT,
            format!("{api}/settings"),
            Some(cookie.clone()),
            Some(json!({"site_url": site})),
        )
        .await;
        println!("SETTINGS {s} {b}");
    }
    for chunk in paths.chunks(5) {
        let (s, _, b) = call(
            Method::POST,
            format!("{api}/scan"),
            Some(cookie.clone()),
            Some(json!({"paths": chunk})),
        )
        .await;
        println!("SCAN {s} {}", serde_json::to_string_pretty(&b).unwrap());
    }
    let (_, _, v) = call(Method::GET, api, Some(cookie), None).await;
    println!("VIEW {}", serde_json::to_string_pretty(&v).unwrap());
}
