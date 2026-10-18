//! Live review end to end (docs/design/live-review-contract.md): member + review-link auth, strict widget CORS,
//! annotations with key resolution, threads, reactions, captures, filters, apply → translation, manual text
//! editing (validate, revert, delete, create key), feedback conversion, SSE replay, shared rate limit.
//! Skipped when Postgres is unreachable.
use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use tower::ServiceExt;

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> (StatusCode, HeaderMap, Bytes) {
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let (status, h) = (res.status(), res.headers().clone());
    (
        status,
        h,
        res.into_body().collect().await.unwrap().to_bytes(),
    )
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
}

#[tokio::test]
async fn live_review() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://glot:glot@localhost:5433/glot".into());
    let Ok(admin) = PgPool::connect(&url).await else {
        eprintln!("SKIPPED: postgres unreachable at {url}");
        return;
    };
    // SAFETY: set before any server code reads it; this test binary has a single test.
    unsafe { std::env::set_var("GLOT_REVIEW_RATE_PER_MIN", "60") };
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

    // ---- member (cookie) ----
    let (_, h, b) = send(
        &app,
        Method::POST,
        "/api/auth/signup",
        &[("content-type", "application/json")],
        Body::from(
            json!({"email": "o@example.com", "password": "correct horse", "name": "Olga"})
                .to_string(),
        ),
    )
    .await;
    let me: Value = serde_json::from_slice(&b).unwrap();
    let me_id = me["id"].as_str().unwrap().to_string();
    let cookie = h[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let m = |method: Method, uri: &str, body: Value| {
        let (app, cookie, uri) = (app.clone(), cookie.clone(), uri.to_string());
        async move {
            let body = if body.is_null() {
                Body::empty()
            } else {
                Body::from(body.to_string())
            };
            let (s, _, b) = send(
                &app,
                method,
                &uri,
                &[("content-type", "application/json"), ("cookie", &cookie)],
                body,
            )
            .await;
            (
                s,
                serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null),
            )
        }
    };
    let (s, _) = m(
        Method::POST,
        "/api/projects",
        json!({"name": "R", "slug": "rv", "source_locale": "en", "target_locales": ["fr"]}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    for (k, src, fr) in [
        ("home.title", "Welcome back", "Bon retour"),
        ("home.greet", "Hello {name}!", "Salut {name} !"),
    ] {
        m(
            Method::POST,
            "/api/projects/rv/keys",
            json!({"key": k, "source": src}),
        )
        .await;
        let (s, v) = m(
            Method::PUT,
            &format!("/api/projects/rv/keys/{k}/translations/fr"),
            json!({"value": fr}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    let r = "/api/projects/rv/review";

    // ---- settings: declared origins ----
    let (s, _) = m(
        Method::PUT,
        &format!("{r}/settings"),
        json!({"origins": ["https://a.b/path"]}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = m(
        Method::PUT,
        &format!("{r}/settings"),
        json!({"origins": ["HTTPS://Shop.example/"]}),
    )
    .await;
    assert_eq!(
        (s, &v["origins"]),
        (StatusCode::OK, &json!(["https://shop.example"]))
    );

    // ---- member annotation: key resolved from the rendered text, route normalized, body sanitized ----
    let anchor = json!({"selector": "main > h1", "text": "Bon  retour", "url": "https://shop.example/fr/home/?u=a@b.c#x",
        "locale": "fr", "viewport": {"w": 1280, "h": 800, "dpr": 2}, "rect": {"x": 10, "y": 20, "w": 300, "h": 40},
        "point": {"x": 0.5, "y": 0.5}, "evil": "<img src=x onerror=alert(1)>"});
    let (s, a1) = m(
        Method::POST,
        &format!("{r}/annotations"),
        json!({"anchor": anchor, "body": "Too **formal** <script>x</script>"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a1}");
    assert_eq!(
        (a1["key"].as_str(), a1["route"].as_str(), a1["url"].as_str()),
        (
            Some("home.title"),
            Some("/home"),
            Some("https://shop.example/fr/home/")
        )
    );
    assert_eq!(a1["translation"]["value"], "Bon retour");
    assert_eq!(
        a1["body_html"],
        "Too <strong>formal</strong> &lt;script&gt;x&lt;/script&gt;"
    );
    assert!(a1["anchor"].get("evil").is_none());
    assert_eq!(a1["status"], "open");
    let a1_id = a1["id"].as_str().unwrap().to_string();

    // ---- review links ----
    let (s, l) = m(
        Method::POST,
        &format!("{r}/links"),
        json!({"label": "Agency", "access": "comment"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{l}");
    let tok = l["token"].as_str().unwrap().to_string();
    assert!(tok.starts_with("glr_") && l["link"]["expires_at"].is_string());
    let (_, ro) = m(
        Method::POST,
        &format!("{r}/links"),
        json!({"access": "read", "expires_in_days": null}),
    )
    .await;
    let ro_tok = ro["token"].as_str().unwrap().to_string();
    let (_, ed) = m(
        Method::POST,
        &format!("{r}/links"),
        json!({"access": "edit"}),
    )
    .await;
    let ed_tok = ed["token"].as_str().unwrap().to_string();

    let w = |method: Method, uri: &str, token: &str, origin: &str, body: Value| {
        let (app, uri, auth, origin) = (
            app.clone(),
            format!("/api/review/w{uri}"),
            format!("Bearer {token}"),
            origin.to_string(),
        );
        async move {
            let body = if body.is_null() {
                Body::empty()
            } else {
                Body::from(body.to_string())
            };
            let mut hs = vec![
                ("content-type", "application/json"),
                ("authorization", auth.as_str()),
            ];
            if !origin.is_empty() {
                hs.push(("origin", origin.as_str()));
            }
            let (s, h, b) = send(&app, method, &uri, &hs, body).await;
            (
                s,
                h,
                serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null),
            )
        }
    };
    let shop = "https://shop.example";
    let (s, h, v) = w(Method::GET, "/context", &tok, shop, Value::Null).await;
    assert_eq!(
        (s, v["access"].as_str(), v["project"]["slug"].as_str()),
        (StatusCode::OK, Some("comment"), Some("rv"))
    );
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], shop);
    let (s, h, v) = w(
        Method::GET,
        "/context",
        &tok,
        "https://evil.example",
        Value::Null,
    )
    .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("origin_not_allowed"))
    );
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    let (_, h, _) = send(
        &app,
        Method::OPTIONS,
        "/api/review/w/annotations",
        &[
            ("origin", shop),
            ("access-control-request-method", "POST"),
            (
                "access-control-request-headers",
                "authorization,content-type",
            ),
        ],
        Body::empty(),
    )
    .await;
    assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], shop);
    let (_, h, _) = send(
        &app,
        Method::OPTIONS,
        "/api/review/w/annotations",
        &[
            ("origin", "https://evil.example"),
            ("access-control-request-method", "POST"),
        ],
        Body::empty(),
    )
    .await;
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    let (s, _, _) = w(Method::GET, "/context", "glr_nope", "", Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // reviewer without account: name required, then annotate an unknown text (free annotation, key = null)
    let free = json!({"selector": "#hero p", "text": "Texte dynamique", "url": "https://shop.example/fr/pricing", "locale": "fr", "viewport": {"w": 390, "h": 844}});
    let (s, _, _) = w(
        Method::POST,
        "/annotations",
        &tok,
        shop,
        json!({"anchor": free, "body": "hm"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, a2) = w(
        Method::POST,
        "/annotations",
        &tok,
        shop,
        json!({"anchor": free, "body": "Typo here", "author_name": "Ana"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{a2}");
    assert_eq!(
        (
            a2["key"].clone(),
            a2["author"]["name"].as_str(),
            a2["route"].as_str()
        ),
        (Value::Null, Some("Ana"), Some("/pricing"))
    );
    let a2_id = a2["id"].as_str().unwrap().to_string();

    // thread + mentions filtered to members, status needs edit access, reactions/approval
    let (s, _, c) = w(Method::POST, &format!("/annotations/{a1_id}/comments"), &tok, shop,
        json!({"body": "Agree, see [doc](https://x.y/z)", "mentions": [me_id, uuid::Uuid::new_v4()], "author_name": "Ana"})).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    assert_eq!(c["mentions"], json!([me_id]));
    assert!(
        c["body_html"]
            .as_str()
            .unwrap()
            .contains("<a href=\"https://x.y/z\" rel=\"noopener noreferrer nofollow\"")
    );
    let cid = c["id"].as_str().unwrap().to_string();
    let (s, _, _) = w(
        Method::PATCH,
        &format!("/annotations/{a1_id}"),
        &tok,
        shop,
        json!({"status": "resolved", "author_name": "Ana"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = w(
        Method::PATCH,
        &format!("/annotations/{a1_id}/comments/{cid}"),
        &tok,
        shop,
        json!({"body": "edited", "author_name": "Bob"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = w(
        Method::PATCH,
        &format!("/annotations/{a1_id}/comments/{cid}"),
        &tok,
        shop,
        json!({"body": "edited", "author_name": "Ana"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, v) = w(
        Method::POST,
        &format!("/annotations/{a1_id}/reactions"),
        &tok,
        shop,
        json!({"emoji": "approve", "author_name": "Ana"}),
    )
    .await;
    assert_eq!(
        (s, &v["reactions"]),
        (
            StatusCode::OK,
            &json!([{"emoji": "approve", "count": 1, "mine": true}])
        )
    );
    let (s, _, _) = w(
        Method::POST,
        &format!("/annotations/{a1_id}/reactions"),
        &tok,
        shop,
        json!({"emoji": "<b>", "author_name": "Ana"}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // captures: PNG ok (dimensions read), SVG refused, served back with nosniff
    let cap = |id: &str, bytes: Vec<u8>| {
        let (app, uri) = (
            app.clone(),
            format!("/api/review/w/annotations/{id}/attachments?author_name=Ana"),
        );
        let auth = format!("Bearer {tok}");
        async move {
            let (s, _, b) = send(
                &app,
                Method::POST,
                &uri,
                &[("content-type", "image/png"), ("authorization", &auth)],
                Body::from(bytes),
            )
            .await;
            (
                s,
                serde_json::from_slice::<Value>(&b).unwrap_or(Value::Null),
            )
        }
    };
    let (s, att) = cap(&a2_id, png(320, 120)).await;
    assert_eq!(
        (s, att["width"].as_i64(), att["content_type"].as_str()),
        (StatusCode::CREATED, Some(320), Some("image/png")),
        "{att}"
    );
    let (s, _) = cap(&a2_id, b"<svg onload=alert(1)>".to_vec()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = cap(&a2_id, vec![0x89; 1024 * 1024 + 10]).await;
    assert!(
        s == StatusCode::PAYLOAD_TOO_LARGE || s == StatusCode::BAD_REQUEST,
        "{s}"
    );
    let (s, h, _) = send(
        &app,
        Method::GET,
        &format!(
            "/api/review/w/annotations/{a2_id}/attachments/{}?token={tok}",
            att["id"].as_str().unwrap()
        ),
        &[],
        Body::empty(),
    )
    .await;
    assert_eq!(
        (
            s,
            h[header::CONTENT_TYPE].to_str().unwrap(),
            h["x-content-type-options"].to_str().unwrap()
        ),
        (StatusCode::OK, "image/png", "nosniff")
    );

    // read-only link can read, not write
    let (s, _, v) = w(Method::GET, "/annotations", &ro_tok, "", Value::Null).await;
    assert_eq!((s, v["total"].as_i64()), (StatusCode::OK, Some(2)));
    let (s, _, _) = w(
        Method::POST,
        "/annotations",
        &ro_tok,
        "",
        json!({"anchor": free, "body": "x", "author_name": "Z"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // ---- member: status, assignment, filters ----
    let (s, v) = m(Method::PATCH, &format!("{r}/annotations/{a1_id}"), json!({"status": "in_progress", "assignee_id": me_id, "suggestion": "Content de vous revoir"})).await;
    assert_eq!(
        (s, v["status"].as_str(), v["assignee"]["name"].as_str()),
        (StatusCode::OK, Some("in_progress"), Some("Olga")),
        "{v}"
    );
    let (s, _) = m(
        Method::PATCH,
        &format!("{r}/annotations/{a1_id}"),
        json!({"assignee_id": uuid::Uuid::new_v4()}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    for (q, n) in [
        ("route=/home&locale=fr", 1),
        ("status=open,in_progress", 2),
        ("status=resolved", 0),
        ("key=home.title", 1),
        ("author=Ana", 1),
        (&format!("author={me_id}"), 1),
        ("assignee=me", 1),
        ("q=typo", 1),
        ("locale=de", 0),
    ] {
        let (s, v) = m(Method::GET, &format!("{r}/annotations?{q}"), Value::Null).await;
        assert_eq!(
            (s, v["total"].as_i64()),
            (StatusCode::OK, Some(n)),
            "{q}: {v}"
        );
    }
    let (s, _) = m(
        Method::GET,
        &format!("{r}/annotations?status=bogus"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, one) = m(
        Method::GET,
        &format!("{r}/annotations/{a1_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (
            one["comments"][0]["body"].as_str(),
            one["approvals"].as_i64()
        ),
        (Some("edited"), Some(1))
    );

    // ---- apply the suggestion → validated translation, annotation resolved ----
    let (s, v) = m(
        Method::POST,
        &format!("{r}/annotations/{a2_id}/apply"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("no_key"))
    );
    let (s, v) = m(
        Method::POST,
        &format!("{r}/annotations/{a1_id}/apply"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (s, v["status"].as_str(), v["translation"]["value"].as_str()),
        (
            StatusCode::OK,
            Some("resolved"),
            Some("Content de vous revoir")
        ),
        "{v}"
    );
    assert!(v["resolved_at"].is_string());
    let (_, hist) = m(
        Method::GET,
        "/api/projects/rv/keys/home.title/history",
        Value::Null,
    )
    .await;
    assert_eq!(
        (hist[0]["value"].as_str(), hist[0]["author"].as_str()),
        (Some("Content de vous revoir"), Some("Olga"))
    );

    // ---- manual text editing (no AI) ----
    let t = format!("{r}/text/home.greet");
    let (s, v) = m(Method::GET, &t, Value::Null).await;
    assert_eq!(
        (s, v["source"].as_str(), v["values"]["fr"]["value"].as_str()),
        (
            StatusCode::OK,
            Some("Hello {name}!"),
            Some("Salut {name} !")
        )
    );
    let (s, v) = m(Method::PUT, &t, json!({"values": {"fr": "Salut !"}})).await;
    assert_eq!(
        (s, v["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("invalid_translation")
        ),
        "{v}"
    );
    assert!(
        !v["error"]["details"]["results"]["fr"]["issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (s, v) = m(
        Method::PUT,
        &t,
        json!({"values": {"fr": "Salut !"}, "dry_run": true}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = m(Method::PUT, &t, json!({"values": {"de": "Hallo {name}!"}})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let fit = json!({"max_width_px": 4000.0, "max_lines": 1, "font": {"family": "Arial", "size_px": 14.0, "weight": 400}});
    let (s, v) = m(
        Method::PUT,
        &t,
        json!({"values": {"fr": "Coucou {name} !"}, "fit": fit}),
    )
    .await;
    assert_eq!(
        (s, v["results"]["fr"]["translation"]["value"].as_str()),
        (StatusCode::OK, Some("Coucou {name} !")),
        "{v}"
    );
    // the edit link can edit too
    let (s, _, v) = w(
        Method::POST,
        "/text/home.greet/revert",
        &ed_tok,
        "",
        json!({"locale": "fr", "author_name": "Ed"}),
    )
    .await;
    assert_eq!(
        (s, v["translation"]["value"].as_str()),
        (StatusCode::OK, Some("Salut {name} !")),
        "{v}"
    );
    let (s, _, _) = w(
        Method::POST,
        "/text/home.greet/revert",
        &tok,
        shop,
        json!({"locale": "fr", "author_name": "Ana"}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = m(Method::DELETE, &format!("{t}/translations/fr"), Value::Null).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = m(Method::GET, &t, Value::Null).await;
    assert_eq!(v["values"]["fr"]["status"], "missing");
    let (s, v) = m(
        Method::POST,
        &format!("{t}/revert"),
        json!({"locale": "fr"}),
    )
    .await;
    assert_eq!(
        (s, v["translation"]["value"].as_str()),
        (StatusCode::OK, Some("Salut {name} !")),
        "{v}"
    );
    let (s, v) = m(Method::GET, &format!("{t}/history?locale=fr"), Value::Null).await;
    assert_eq!((s, v[0]["author"].as_str()), (StatusCode::OK, Some("Olga")));
    let (s, v) = m(
        Method::POST,
        &format!("{r}/text"),
        json!({"source": "Start free trial", "anchor": free, "values": {"fr": "Essai gratuit"}}),
    )
    .await;
    assert_eq!(
        (s, v["key"].as_str(), v["values"]["fr"]["status"].as_str()),
        (
            StatusCode::CREATED,
            Some("pricing.start_free_trial"),
            Some("reviewed")
        ),
        "{v}"
    );
    let (_, v) = m(
        Method::POST,
        &format!("{r}/text"),
        json!({"source": "Start free trial", "anchor": free}),
    )
    .await;
    assert_eq!(v["key"], "pricing.start_free_trial_2");
    let (s, _) = m(
        Method::POST,
        &format!("{r}/text"),
        json!({"source": "x", "key": "home.title"}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // ---- feedback report → annotation ----
    let (_, fs) = m(
        Method::PUT,
        "/api/projects/rv/feedback/settings",
        json!({"threshold": 50}),
    )
    .await;
    let pk = fs["public_key"].as_str().unwrap();
    let (s, _) = m(Method::POST, &format!("/api/feedback/{pk}"), json!({"locale": "fr", "text": "Salut Ana !", "kind": "awkward",
        "suggestion": "Bonjour Ana !", "url": "https://shop.example/fr/home?x=1", "dom": "main>h2"})).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (_, fb) = m(Method::GET, "/api/projects/rv/feedback", Value::Null).await;
    let gid = fb["groups"][0]["id"].as_str().unwrap().to_string();
    let (s, v) = m(
        Method::POST,
        &format!("{r}/from-feedback/{gid}"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (
            s,
            v["key"].as_str(),
            v["route"].as_str(),
            v["suggestion"].as_str()
        ),
        (
            StatusCode::CREATED,
            Some("home.greet"),
            Some("/home"),
            Some("Bonjour Ana !")
        ),
        "{v}"
    );
    let (s, _) = m(
        Method::POST,
        &format!("{r}/from-feedback/{gid}"),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // ---- history + SSE replay ----
    let (_, h) = m(
        Method::GET,
        &format!("{r}/annotations/{a1_id}/history"),
        Value::Null,
    )
    .await;
    let kinds: Vec<&str> = h
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "annotation.created",
            "comment.created",
            "comment.updated",
            "reaction.toggled",
            "annotation.updated",
            "suggestion.applied"
        ]
    );
    let req = Request::get(format!("/api/review/w/events?since=0&token={ro_tok}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
    let mut body = res.into_body();
    let mut text = String::new();
    while !text.contains("feedback_group_id") {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("sse backlog")
            .unwrap()
            .unwrap();
        text.push_str(std::str::from_utf8(frame.data_ref().unwrap()).unwrap());
    }
    assert!(
        text.contains("event: review") && text.contains("\"kind\":\"text.created\""),
        "{text}"
    );
    // live: an event written now reaches the open stream through LISTEN/NOTIFY
    let before = text.matches("comment.created").count();
    let (s, _) = m(
        Method::POST,
        &format!("{r}/annotations/{a1_id}/comments"),
        json!({"body": "live!"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    while text.matches("comment.created").count() == before {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), body.frame())
            .await
            .expect("live event")
            .unwrap()
            .unwrap();
        if let Some(d) = frame.data_ref() {
            text.push_str(std::str::from_utf8(d).unwrap());
        }
    }

    // ---- author deletes, revocation, rate limit ----
    let (s, _, _) = w(
        Method::DELETE,
        &format!("/annotations/{a2_id}?author_name=Bob"),
        &tok,
        shop,
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = w(
        Method::DELETE,
        &format!("/annotations/{a2_id}?author_name=Ana"),
        &tok,
        shop,
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let mut limited = false;
    for _ in 0..70 {
        let (s, h, _) = w(
            Method::POST,
            &format!("/annotations/{a1_id}/reactions"),
            &tok,
            shop,
            json!({"emoji": "eyes", "author_name": "Ana"}),
        )
        .await;
        if s == StatusCode::TOO_MANY_REQUESTS {
            assert!(h.get(header::RETRY_AFTER).is_some());
            limited = true;
            break;
        }
    }
    assert!(limited);
    let (s, _) = m(
        Method::DELETE,
        &format!("{r}/links/{}", l["link"]["id"].as_str().unwrap()),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _, _) = w(Method::GET, "/context", &tok, shop, Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // member widget session
    let (s, v) = m(Method::POST, &format!("{r}/session"), Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, v) = w(
        Method::GET,
        "/context",
        v["token"].as_str().unwrap(),
        "http://localhost:3100",
        Value::Null,
    )
    .await;
    assert_eq!(
        (s, v["access"].as_str(), v["me"]["name"].as_str()),
        (StatusCode::OK, Some("member"), Some("Olga"))
    );

    pool.close().await;
    let _ = sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await;
}
