//! Translating under a pixel budget: a too-wide translation is rejected by the validator and the
//! retry loop gets it shortened *before* anything is written (mock provider, real font metrics).
use std::fs;
use std::path::Path;

use glot_core::measure::{Fit, Font, Layout, measure_text, save_layout};
use glot_core::validate::{IssueCode, Syntax, render_sample};
use glot_llm::engine::{TranslateOptions, Workspace};

#[tokio::test]
async fn too_wide_translation_is_rejected_then_shortened() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("glot.toml"),
        "source_locale = \"en\"\ntarget_locales = [\"de\"]\nfiles = [\"messages/{locale}.json\"]\n\n[llm]\nprovider = \"mock\"\nmodel = \"mock\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("messages")).unwrap();
    fs::write(
        root.join("messages/en.json"),
        r#"{"invoice":{"mark_paid":"Mark as paid","greet":"Hello {name}"}}"#,
    )
    .unwrap();
    let fonts = root.join(".glot/fonts");
    fs::create_dir_all(&fonts).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/Geist-Regular.ttf");
    fs::copy(fixture, fonts.join("geist.ttf")).unwrap();

    let font = Font {
        family: "Geist".into(),
        postscript: Some("Geist-Regular".into()),
        size_px: 14.0,
        weight: 400,
        letter_spacing_px: 0.0,
        text_transform: None,
    };
    glot_core::measure::register_fonts_dir(&fonts);
    let source_px = measure_text("Mark as paid", &font);
    // The source fits with 10 % to spare; the mock's "⟦de⟧ Mark as paid" does not.
    let layout = Layout::from([(
        "invoice.mark_paid".to_string(),
        Fit {
            max_width_px: (source_px * 1.1).round(),
            max_lines: Some(1),
            font: font.clone(),
            text: Some("Mark as paid".into()),
            text_px: Some(source_px),
            occurrences: vec![],
        },
    )]);
    save_layout(root, &layout).unwrap();
    let fit = &layout["invoice.mark_paid"];
    assert!(
        glot_core::measure::check("⟦de⟧ Mark as paid", Syntax::Icu, fit)
            .is_some_and(|i| i.code == IssueCode::TooWide),
        "the first mock answer must be too wide"
    );

    let mut ws = Workspace::open(root).unwrap();
    let report = ws.translate(TranslateOptions::default()).await.unwrap();
    assert!(report.rejected.is_empty(), "{:#?}", report.rejected);
    let t = report.written["de"]
        .iter()
        .find(|t| t.key == "invoice.mark_paid")
        .unwrap();
    assert!(t.attempts >= 2, "rejected at least once: {t:?}");
    assert!(
        measure_text(&render_sample(&t.value, Syntax::Icu), &font) <= fit.max_width_px,
        "{:?} still too wide",
        t.value
    );
    assert!(!glot_core::validate::has_errors(&t.issues), "{t:?}");
    // Keys without a measure keep the length-ratio heuristic; nothing too wide on disk.
    assert!(ws.check().iter().all(|f| f.issue.code != IssueCode::TooWide
        || f.issue.severity == glot_core::validate::Severity::Warning));
    let de = fs::read_to_string(root.join("messages/de.json")).unwrap();
    assert!(de.contains(&t.value), "{de}");
}
