//! `glot_core::validate` + glossary check for the browser.
//! ponytail: raw C ABI (alloc/check/free) instead of wasm-bindgen: zero extra deps, ~20 lines of JS glue.
//! Protocol: JS writes a UTF-8 JSON `Input` with `alloc`, calls `check(ptr, len)`, which returns a pointer to
//! `[len: u32 LE][UTF-8 JSON Vec<Issue>]`; JS then frees both buffers with `dealloc`.
use glot_core::glossary::{Glossary, Term};
use glot_core::validate::{Constraints, Issue, IssueCode, Severity, Syntax, validate};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Input {
    pub source: String,
    pub target: String,
    pub locale: String,
    #[serde(default)]
    pub syntax: Syntax,
    #[serde(default)]
    pub glossary: Vec<Term>,
    #[serde(default)]
    pub max_length_ratio: Option<f32>,
}

pub fn check_json(input: &str) -> String {
    let issues = match serde_json::from_str::<Input>(input) {
        Ok(i) => {
            let c = Constraints {
                max_length: None,
                max_length_ratio: i.max_length_ratio,
            };
            let mut out = validate(&i.source, &i.target, &i.locale, i.syntax, &c);
            out.extend(Glossary { terms: i.glossary }.check(&i.source, &i.target, &i.locale));
            out
        }
        Err(e) => vec![Issue {
            code: IssueCode::SyntaxError,
            severity: Severity::Error,
            message: format!("bad input: {e}"),
        }],
    };
    serde_json::to_string(&issues).unwrap()
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

/// # Safety
/// `ptr`/`cap` must come from `alloc(cap)` (or the buffer returned by `check`, with cap = 4 + its length).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, cap: usize) {
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, cap)) });
}

/// # Safety
/// `ptr..ptr+len` must be a buffer obtained from `alloc` holding UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn check(ptr: *const u8, len: usize) -> *mut u8 {
    let input = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) });
    let out = check_json(&input);
    let mut buf = Vec::new();
    buf.extend_from_slice(&(out.len() as u32).to_le_bytes());
    buf.extend_from_slice(out.as_bytes());
    Box::into_raw(buf.into_boxed_slice()) as *mut u8
}

#[cfg(test)]
mod tests {
    use super::check_json;

    #[test]
    fn checks() {
        let ok = check_json(r#"{"source":"Hi {name}","target":"Salut {name}","locale":"fr"}"#);
        assert_eq!(ok, "[]");
        let bad = check_json(
            r#"{"source":"Open glot","target":"Ouvre l'outil","locale":"fr","glossary":[{"term":"glot","do_not_translate":true}]}"#,
        );
        assert!(bad.contains("do_not_translate_violation"), "{bad}");
        assert!(
            check_json(r#"{"source":"{n}","target":"x","locale":"fr"}"#)
                .contains("placeholder_missing")
        );
    }
}
