//! `glot labs distill`: dataset from reviewed pairs, RAG prompts, eval lanes, local translation (mock).
use std::fs;

use glot_core::lock::Origin;
use glot_llm::{
    distill::{self, Lane},
    engine::Workspace,
    mock::MockProvider,
};

const EN: &str = r#"{"a":"Save changes","b":"Delete workspace","c":"Invite members to your workspace","d":"Delete project","e":"Your API key is sk-live-0123456789abcdefXYZ","f":"Billing settings","g":"{n, plural, one {# file} other {# files}}","new":"Delete this workspace and all its data"}"#;

#[tokio::test]
async fn dataset_rag_eval_translate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"fr\"]\nfiles = [\"messages/{locale}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\nstyle = \"Friendly, vous\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("messages")).unwrap();
    fs::write(root.join("messages/en.json"), EN).unwrap();
    fs::write(
        root.join("glossary.toml"),
        "[[term]]\nterm = \"workspace\"\ntranslations = { fr = \"espace de travail\" }\n",
    )
    .unwrap();
    let mut ws = Workspace::open(root).unwrap();
    for (k, v) in [
        ("a", "Enregistrer les modifications"),
        ("b", "Supprimer l'espace de travail"),
        ("c", "Inviter des membres dans votre espace de travail"),
        ("d", "Supprimer le projet"),
        ("e", "Votre clé API est sk-live-0123456789abcdefXYZ"),
        ("f", "Paramètres de facturation"),
        (
            "g",
            "{n, plural, one {# fichier} many {# fichiers} other {# fichiers}}",
        ),
    ] {
        let issues = ws.set_translation("fr", k, v).unwrap();
        assert!(!glot_core::validate::has_errors(&issues), "{k}: {issues:?}");
    }
    // A machine translation that nobody reviewed is not part of the dataset.
    ws.lock
        .entries
        .get_mut("fr")
        .unwrap()
        .get_mut("f")
        .unwrap()
        .origin = Origin::Machine { model: "m".into() };
    ws.lock
        .entries
        .get_mut("fr")
        .unwrap()
        .get_mut("f")
        .unwrap()
        .reviewed = false;

    let ds = distill::collect(&ws, 30);
    assert_eq!(ds.skipped_secret, 1);
    assert_eq!(ds.train.len() + ds.eval.len(), 5);
    assert!(!ds.eval.is_empty() && !ds.train.is_empty());
    let all: Vec<_> = ds.train.iter().chain(&ds.eval).collect();
    assert!(all.iter().all(|p| p.key != "e" && p.key != "f"));

    // Fine-tuning JSONL: system (style + glossary) / user / assistant.
    let out = root.join(distill::DIR);
    distill::write(&ws, &ds, &out).unwrap();
    let train = fs::read_to_string(out.join("train.jsonl")).unwrap();
    assert_eq!(train.lines().count(), ds.train.len());
    for line in train.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[2]["role"], "assistant");
        assert!(m[0]["content"].as_str().unwrap().contains("Friendly, vous"));
    }
    assert!(!train.contains("sk-live"));
    let b = distill::chat(&ws, all.iter().find(|p| p.key == "b").unwrap());
    assert!(
        b["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("\"workspace\" => \"espace de travail\"")
    );

    // Retrieval pool never contains the held-out answers.
    let pool = distill::pool(&ws, &ds.train, &ds.eval);
    for e in &ds.eval {
        assert!(pool.iter().all(|p| p.source != e.source));
    }
    let full = distill::pool(&ws, &ds.train, &[]);
    let req = distill::request(
        &ws,
        "fr",
        &[(
            "new".into(),
            "Delete this workspace and all its data".into(),
            None,
        )],
        Some(&full),
        3,
    );
    let desc = req.items[0].context.description.as_deref().unwrap();
    assert!(desc.contains("Approved translations"), "{desc}");
    assert!(desc.contains("Supprimer l'espace de travail"), "{desc}");
    assert_eq!(req.glossary[0].term, "workspace");

    // Eval: three lanes on the mock, judged by the mock.
    let local = distill::local_config(&ws.config.llm, "mock", "qwen-mock", None, false);
    let lanes = [
        Lane {
            name: "local".into(),
            llm: local.clone(),
            rag: false,
        },
        Lane {
            name: "local+rag".into(),
            llm: local.clone(),
            rag: true,
        },
        Lane {
            name: "cloud".into(),
            llm: ws.config.llm.clone(),
            rag: false,
        },
    ];
    let (scores, _) = distill::eval(
        &ws,
        &ds.eval,
        &pool,
        &lanes,
        Some((&MockProvider, "mock")),
        3,
    )
    .await
    .unwrap();
    assert_eq!(scores.len(), 3);
    for s in &scores {
        assert_eq!(s.n, ds.eval.len());
        assert_eq!(s.valid, s.n, "{s:?}");
        assert_eq!(s.cost_usd, 0.0);
        assert_eq!(s.judge, Some(100.0));
    }
    let (text, block) = distill::recommend(&scores, &local, &ws.config.llm);
    assert!(text.contains("100 %"), "{text}");
    distill::write_proposal(root, &block).unwrap();
    assert!(
        Workspace::open(root).is_ok(),
        "proposal must keep glot.toml valid"
    );

    // Translate: local first pass + RAG, written through the TM (no other LLM call).
    let o = distill::translate(&mut ws, &local, 3, None, Some("sentence"))
        .await
        .unwrap();
    assert_eq!(o.local, 1, "only 'new' (sentence) is missing: {o:?}");
    let ws = Workspace::open(root).unwrap();
    assert!(ws.catalogs["fr"].entries["new"].contains("espace de travail"));
    assert_eq!(
        ws.lock.entries["fr"]["new"].origin,
        Origin::Machine { model: "tm".into() }
    );
}
