//! Live stream translation over HTTP: opt-in, the resumable cursor, translation memory, glossary, what is
//! stored (never the text unless asked, never personal data), browser access, daily cap.
//! Real Postgres (DATABASE_URL, default :5433), skipped when unreachable. Mock provider (`⟦fr⟧ …`).
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::time::Duration;
use tower::ServiceExt;

/// `(event, data)` pairs of an SSE body.
fn sse(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter_map(|block| {
            let event = block.lines().find_map(|l| l.strip_prefix("event:"))?.trim();
            let data = block.lines().find_map(|l| l.strip_prefix("data:"))?;
            Some((event.to_string(), serde_json::from_str(data.trim()).ok()?))
        })
        .collect()
}

fn text(events: &[(String, Value)]) -> String {
    events
        .iter()
        .filter(|e| e.0 == "delta")
        .map(|e| e.1["text"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn live_stream_end_to_end() {
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
    let state = glot_server::AppState::new(pool.clone(), "test-secret", false);
    let app = glot_server::router(state, "http://localhost:3100");

    let call = |method: Method, uri: &str, cookie: Option<&str>, body: Option<Value>| {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, format!("glot_session={c}"));
        }
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let req = req
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let (status, headers) = (res.status(), res.headers().clone());
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (status, headers, String::from_utf8_lossy(&bytes).to_string())
        }
    };
    let json_of = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Null);

    let (s, h, _) = call(
        Method::POST,
        "/api/auth/signup",
        None,
        Some(json!({"email": "o@x.io", "password": "correct horse", "name": "O"})),
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
    call(
        Method::POST,
        "/api/projects",
        c,
        Some(json!({"name": "L", "slug": "ls", "source_locale": "en", "target_locales": ["fr"]})),
    )
    .await;
    call(
        Method::PATCH,
        "/api/projects/ls",
        c,
        Some(json!({"llm": {"provider": "mock", "model": "mock"}})),
    )
    .await;
    let (s, _, b) = call(
        Method::POST,
        "/api/projects/ls/glossary",
        c,
        Some(json!({"term": "workspace", "translations": {"fr": "espace de travail"}, "case_sensitive": true})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    call(
        Method::POST,
        "/api/projects/ls/keys",
        c,
        Some(json!({"key": "home.title", "source": "Welcome back."})),
    )
    .await;
    sqlx::query(
        "INSERT INTO translations (key_id, locale, value, status, origin, source_hash)
         SELECT id, 'fr', 'Bon retour parmi nous.', 'reviewed', 'human', 'x' FROM keys WHERE key = 'home.title'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let api = "/api/projects/ls/live-stream";
    let tr = format!("{api}/translate");

    // Off until an admin opts in, and the refusal says where to do it.
    let (s, _, b) = call(
        Method::POST,
        &tr,
        c,
        Some(json!({"to": "fr", "text": "Hello."})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(json_of(&b)["error"]["code"], "live_stream_disabled");
    let (_, _, b) = call(Method::GET, api, c, None).await;
    let st = json_of(&b);
    assert_eq!(
        st["settings"],
        json!({"enabled": false, "browser": false, "mask_pii": true, "store": false, "model": null, "daily_chars": 500000})
    );
    assert_eq!(
        st["provider"]["ready"], false,
        "the mock provider is not a translation: the dashboard says so"
    );
    assert_eq!(st["days"], json!([]));
    let pk = st["public_key"].as_str().unwrap().to_string();

    let put = |v: Value| call(Method::PUT, api, c, Some(v));
    let on = json!({"enabled": true, "browser": false, "mask_pii": true, "store": false, "model": null, "daily_chars": 500000});
    let (s, _, b) = put(on.clone()).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Turn 1, first request: the stream so far ends in the middle of a sentence.
    let part1 = "## Setup\nWelcome back. Mail léa@exemple.fr about your workspace. Then run `glot push` and wai";
    let (s, h, b) = call(
        Method::POST,
        &tr,
        c,
        Some(json!({"to": "fr", "from": "en", "text": part1, "conversation": "c1"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        h[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let ev = sse(&b);
    assert_eq!(
        text(&ev),
        "## ⟦fr⟧ Setup\nBon retour parmi nous. ⟦fr⟧ Mail léa@exemple.fr about your espace de travail. "
    );
    let via: Vec<&str> = ev
        .iter()
        .filter(|e| e.0 == "delta")
        .map(|e| e.1["via"].as_str().unwrap())
        .collect();
    assert_eq!(via, ["raw", "llm", "raw", "tm", "raw", "llm", "raw"]);
    let (_, done) = ev.last().unwrap();
    let consumed = done["consumed"].as_u64().unwrap() as usize;
    let utf16: Vec<u16> = part1.encode_utf16().collect();
    let rest = String::from_utf16(&utf16[consumed..]).unwrap();
    assert_eq!(
        rest, "Then run `glot push` and wai",
        "the unfinished sentence is left to the caller"
    );
    assert_eq!(done["mid"], true);
    assert_eq!(done["fence"], Value::Null);
    assert_eq!(
        ev.iter()
            .filter(|e| e.0 == "delta")
            .map(|e| e.1["n"].as_u64().unwrap() as usize)
            .sum::<usize>(),
        consumed
    );

    // Second request: the rest + what arrived since, resumed from `mid`; `final` flushes.
    let part2 = format!("{rest}t.\n```sh\necho \"Hello. World\"\n```\nWelcome back.");
    let (_, _, b) = call(
        Method::POST,
        &tr,
        c,
        Some(json!({"to": "fr", "from": "en", "text": part2, "conversation": "c1", "mid": true, "final": true})),
    )
    .await;
    let ev = sse(&b);
    assert_eq!(
        text(&ev),
        "⟦fr⟧ Then run `glot push` and wait.\n```sh\necho \"Hello. World\"\n```\nBon retour parmi nous."
    );
    assert_eq!(
        ev.last().unwrap().1["consumed"].as_u64().unwrap() as usize,
        part2.encode_utf16().count()
    );
    assert_eq!(ev.last().unwrap().1["stats"]["units"], 2);

    // Turn 2 of the same conversation: the sentence already said comes back identical, from memory.
    let (_, _, b) = call(
        Method::POST,
        &tr,
        c,
        Some(json!({"to": "fr", "from": "en", "text": "Mail léa@exemple.fr about your workspace.", "conversation": "c1", "final": true})),
    )
    .await;
    let ev = sse(&b);
    assert_eq!(
        text(&ev),
        "⟦fr⟧ Mail léa@exemple.fr about your espace de travail."
    );
    assert_eq!(ev[0].1["via"], "cache");

    // Stored: keyed hashes and counters. No text, no translation, no email.
    tokio::time::sleep(Duration::from_millis(300)).await; // counters are written after the response
    let (rows, with_target): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(target) FROM live_stream_units")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(rows >= 4);
    assert_eq!(with_target, 0);
    let dump: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(u::text, ' '), '') || (SELECT coalesce(string_agg(d::text, ' '), '') FROM live_stream_usage d) FROM live_stream_units u",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for leak in ["exemple", "Mail", "workspace", "Setup"] {
        assert!(!dump.contains(leak), "{leak} was stored");
    }
    let (_, _, b) = call(Method::GET, api, c, None).await;
    let st = json_of(&b);
    assert_eq!(st["days"][0]["requests"], 3);
    assert_eq!(st["days"][0]["llm_units"], 3);
    assert_eq!(
        st["days"][0]["known_units"], 3,
        "2 from the translation memory, 1 from the conversation"
    );
    assert_eq!(st["days"][0]["failed_units"], 0);
    assert!(st["days"][0]["chars"].as_i64().unwrap() > 100);
    assert_eq!(st["units"]["stored"], 0);

    // Opt-in storage: a sentence seen again in ANOTHER conversation is free; what is stored is masked.
    let mut with_store = on.clone();
    with_store["store"] = json!(true);
    put(with_store).await;
    let again =
        json!({"to": "fr", "from": "en", "text": "Write to bob@corp.io today.", "final": true});
    let (_, _, b) = call(Method::POST, &tr, c, Some(again.clone())).await;
    assert_eq!(sse(&b)[0].1["via"], "llm");
    let (_, _, b) = call(Method::POST, &tr, c, Some(json!({"to": "fr", "from": "en", "text": "Write to eve@else.org today.", "final": true}))).await;
    let ev = sse(&b);
    assert_eq!(
        ev[0].1["via"], "cache",
        "same sentence, another address: one stored template serves both"
    );
    assert_eq!(text(&ev), "⟦fr⟧ Write to eve@else.org today.");
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT target FROM live_stream_units WHERE target IS NOT NULL")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(stored, ["⟦fr⟧ Write to <x0/> today."]);
    // Turning storage off forgets it.
    put(on.clone()).await;
    let n: i64 = sqlx::query_scalar("SELECT count(target) FROM live_stream_units")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);

    // Browsers (public key): a separate switch; a wrong key learns nothing.
    let public = |key: &str| {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/live/v1/ls/translate")
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(
                json!({"key": key, "to": "fr", "text": "Hello there.", "final": true}).to_string(),
            ))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (status, String::from_utf8_lossy(&bytes).to_string())
        }
    };
    let (s, b) = public(&pk).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(json_of(&b)["error"]["code"], "live_stream_browser_disabled");
    let mut browser = on.clone();
    browser["browser"] = json!(true);
    put(browser.clone()).await;
    let (s, b) = public(&pk).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(text(&sse(&b)), "⟦fr⟧ Hello there.");
    assert_eq!(public("pk_nope").await.0, StatusCode::NOT_FOUND);

    // Daily cap, and bad input, explain themselves.
    browser["daily_chars"] = json!(10);
    put(browser).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (s, _, b) = call(Method::POST, &tr, c, Some(again)).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(json_of(&b)["error"]["code"], "live_stream_daily_cap");
    put(on).await;
    let (s, _, _) = call(
        Method::POST,
        &tr,
        c,
        Some(json!({"to": "not a locale", "text": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, b) = call(
        Method::POST,
        &tr,
        c,
        Some(json!({"to": "fr", "text": "x".repeat(40_000)})),
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(json_of(&b)["error"]["code"], "text_too_long");

    // An admin can forget everything.
    let (s, _, b) = call(Method::DELETE, &format!("{api}/cache"), c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(json_of(&b)["deleted"].as_u64().unwrap() >= 5);

    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {db} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
}

/// Not a test of the code: the real pipeline run on a COPY of the real projects, printing what it gives.
/// `LIVE_STREAM_REAL_DB=postgres://…/glot_real_live_stream cargo test -p glot-server --test live_stream real_projects -- --ignored --nocapture`
/// Refuses to run when a provider could be called (no paid call): only the projects' own memory translates.
#[tokio::test]
#[ignore]
async fn real_projects_report() {
    let Ok(url) = std::env::var("LIVE_STREAM_REAL_DB") else {
        eprintln!("SKIPPED: set LIVE_STREAM_REAL_DB to a copy of glot_real");
        return;
    };
    assert!(
        url.ends_with("_live_stream"),
        "run on your own copy, never on the reference database"
    );
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::raw_sql(
        include_str!("../migrations/0189_live_stream.sql")
            .replace("CREATE TABLE", "CREATE TABLE IF NOT EXISTS")
            .as_str(),
    )
    .execute(&pool)
    .await
    .unwrap();
    let state = glot_server::AppState::new(pool.clone(), "real-report", false);
    let app = glot_server::router(state, "http://localhost:3100");
    let token = format!("glot_{}", uuid::Uuid::new_v4().simple());
    let hash: String = {
        use sha2::Digest;
        sha2::Sha256::digest(token.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let call = |method: Method, uri: String, body: Option<Value>| {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (status, String::from_utf8_lossy(&bytes).to_string())
        }
    };
    let projects: Vec<(uuid::Uuid, String, Vec<String>)> =
        sqlx::query_as("SELECT id, slug, target_locales FROM projects ORDER BY slug")
            .fetch_all(&pool)
            .await
            .unwrap();
    for (id, slug, locales) in projects {
        sqlx::query(
            "INSERT INTO api_tokens (id, user_id, name, token_hash)
             SELECT gen_random_uuid(), user_id, 'live-stream report', $2 FROM project_members WHERE project_id = $1 AND role = 'owner'
             ON CONFLICT (token_hash) DO NOTHING",
        )
        .bind(id)
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();
        let api = format!("/api/projects/{slug}/live-stream");
        let (s, b) = call(Method::PUT, api.clone(), Some(json!({"enabled": true, "browser": false, "mask_pii": true, "store": false, "model": null, "daily_chars": 50000000}))).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let (_, b) = call(Method::GET, api.clone(), None).await;
        let st: Value = serde_json::from_str(&b).unwrap();
        assert_eq!(
            st["provider"]["ready"], false,
            "a provider key is configured: this report must not call a paid API"
        );
        println!(
            "\n=== {slug}: provider {} ({}), {} glossary terms",
            st["provider"]["name"], st["provider"]["error"], st["glossary_terms"]
        );
        // Real source texts of the project (catalog + glot Edge), short enough to be a message.
        let texts: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT t FROM (SELECT source AS t FROM keys WHERE project_id = $1
                                     UNION ALL SELECT source_text FROM edge_segments WHERE project_id = $1) x
             WHERE length(t) BETWEEN 2 AND 600 AND t ~ '[[:alpha:]]' AND t !~ '[{}<]' ORDER BY t",
        )
        .bind(id)
        .fetch_all(&pool)
        .await
        .unwrap();
        for locale in &locales {
            let (mut units, mut tm, mut pass, mut chars, mut multi, mut masked) =
                (0, 0, 0, 0, 0, 0);
            let mut samples: Vec<String> = vec![];
            // Messages of 20 real texts, one per line, sent in two requests cut mid-stream (the cursor is real).
            for (i, group) in texts.chunks(20).enumerate() {
                let msg = group.join("\n") + "\n";
                let cut = msg.char_indices().nth(msg.chars().count() / 2).unwrap().0;
                let conv = format!("report-{slug}-{locale}-{i}");
                let (s, b) = call(Method::POST, format!("{api}/translate"), Some(json!({"to": locale, "from": "fr", "text": &msg[..cut], "conversation": conv}))).await;
                assert_eq!(s, StatusCode::OK, "{b}");
                let mut ev = sse(&b);
                let done = ev.pop().unwrap().1;
                let consumed = done["consumed"].as_u64().unwrap() as usize;
                let utf16: Vec<u16> = msg.encode_utf16().collect();
                let rest = String::from_utf16(&utf16[consumed..]).unwrap();
                let (_, b) = call(Method::POST, format!("{api}/translate"), Some(json!({"to": locale, "from": "fr", "text": rest, "conversation": conv, "final": true, "mid": done["mid"], "fence": done["fence"]}))).await;
                ev.extend(sse(&b));
                let mut covered = 0;
                for (_, d) in ev.iter().filter(|e| e.0 == "delta") {
                    let text = d["text"].as_str().unwrap();
                    covered += d["n"].as_u64().unwrap() as usize;
                    match d["via"].as_str().unwrap() {
                        "raw" => assert!(msg.contains(text)),
                        "tm" | "cache" => {
                            units += 1;
                            tm += 1;
                            if samples.len() < 4 && text.len() > 40 && i % 7 == 3 {
                                samples.push(format!("    [mémoire] {text}"));
                            }
                        }
                        _ => {
                            units += 1;
                            pass += 1;
                            assert_eq!(d["reason"], "no_provider");
                            assert!(
                                msg.contains(text),
                                "{slug}/{locale}: passed-through text is not the source: {text:?}"
                            );
                        }
                    }
                }
                // Nothing lost or duplicated: the deltas stand for the whole message.
                assert_eq!(
                    covered,
                    msg.encode_utf16().count(),
                    "{slug}/{locale}: message {i} not fully covered"
                );
                chars += msg.chars().count();
                for t in group {
                    let mut seg = glot_llm::live_stream::Segmenter::default();
                    let mut p = seg.push(t);
                    p.extend(seg.finish());
                    let n = p
                        .iter()
                        .filter(|x| matches!(x, glot_llm::live_stream::Piece::Text(_)))
                        .count();
                    if n > 1 {
                        multi += 1;
                    }
                    for x in &p {
                        if let glot_llm::live_stream::Piece::Text(u) = x {
                            let m = glot_llm::live_stream::mask(u, true, &[]);
                            masked += m.spans.len();
                        }
                    }
                }
            }
            println!(
                "  → {locale}: {} textes réels ({chars} caractères), {units} unités ; {tm} servies par la mémoire ({:.1} %), {pass} à traduire par l'IA (passées telles quelles : pas de clé) ; {multi} textes de plusieurs phrases ; {masked} éléments protégés (code, liens, e-mails, numéros)",
                texts.len(),
                100.0 * tm as f64 / units.max(1) as f64
            );
            for s in &samples {
                println!("{s}");
            }
        }
        sqlx::query("DELETE FROM api_tokens WHERE token_hash = $1")
            .bind(&hash)
            .execute(&pool)
            .await
            .unwrap();
    }
}
