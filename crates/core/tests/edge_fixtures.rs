//! Shared glot Edge fixtures: `fixtures/edge/*.html` → `*.segments.json` (also checked by
//! `packages/edge/test/segment.test.ts`). `UPDATE_FIXTURES=1 cargo test -p glot-core --test edge_fixtures`
//! rewrites the expected files; review the diff.
use glot_core::edge::{SegmentOpts, segment, translate_html};
use std::path::Path;

/// Same options in the JS test.
fn opts() -> SegmentOpts {
    let mut o = SegmentOpts::new("en").skip(&[".private"]);
    o.dnt = vec!["ClipOnIt".into()];
    o.pii_allow = vec!["support@cliponit.app".into()];
    o
}

#[test]
fn fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edge");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("html") {
            continue;
        }
        n += 1;
        let html = std::fs::read_to_string(&p).unwrap();
        let segs = segment(&html, &opts());
        let got = serde_json::to_string_pretty(&segs).unwrap() + "\n";
        let want_path = p.with_extension("segments.json");
        if std::env::var_os("UPDATE_FIXTURES").is_some() {
            std::fs::write(&want_path, &got).unwrap();
        }
        let want = std::fs::read_to_string(&want_path).unwrap_or_default();
        assert_eq!(got, want, "{}", want_path.display());

        // Identity translation: the result segments the same (only whitespace inside segments may change).
        let same = translate_html(&html, &opts(), |s| Some(s.text.clone()));
        let again = serde_json::to_string_pretty(&segment(&same, &opts())).unwrap() + "\n";
        assert_eq!(again, got, "{} identity", p.display());
        // Untranslated: byte for byte.
        assert_eq!(translate_html(&html, &opts(), |_| None), html);
    }
    assert!(n >= 4);
}
