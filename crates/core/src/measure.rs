//! Pixel budgets: `.glot/layout.json` (written by `glot measure`) and text width without a browser.
//!
//! `glot measure` renders the source locale and records, per key, the box the text can use
//! (`max_width_px` × `max_lines`) and the font it renders with. Here we shape strings with the same
//! font files (webfonts saved in `.glot/fonts/`, else the system font the browser used) so a
//! translation can be rejected for being too wide *before* it is written.
use rustybuzz::{Face, UnicodeBuffer, Variation, ttf_parser::Tag};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::validate::{Issue, IssueCode, Severity, Syntax, render_sample};
use crate::{Error, Key, Result};

pub const LAYOUT_FILE: &str = ".glot/layout.json";
pub const FONTS_DIR: &str = ".glot/fonts";
/// Within this fraction of the budget: warning (rendering differences, hinting, kerning).
pub const MARGIN: f32 = 0.05;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Font {
    /// Family the browser actually rendered with ("Inter", "System Font").
    pub family: String,
    /// PostScript name of that font: finds the file in `.glot/fonts/` or the system fonts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postscript: Option<String>,
    pub size_px: f32,
    #[serde(default = "regular")]
    pub weight: u16,
    #[serde(default)]
    pub letter_spacing_px: f32,
    /// CSS `text-transform` when not `none` (`uppercase` labels are much wider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_transform: Option<String>,
}

fn regular() -> u16 {
    400
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    /// Width the text may take before it overflows its box (padding and icons excluded).
    pub max_width_px: f32,
    /// Lines allowed (line-clamp, nowrap = 1, fixed height). `None`: wraps freely, only each word must fit.
    pub max_lines: Option<u32>,
    pub font: Font,
    /// Source text as rendered and its width in the browser: calibrates the measure here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_px: Option<f32>,
    /// Every place the key rendered (`glot measure`); the budget above is the tightest of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrences: Vec<Occurrence>,
}

/// One rendering of a key: route × viewport × element, with its own box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Occurrence {
    pub route: String,
    pub viewport: String,
    /// CSS path of the element (`main > div:nth-of-type(2) > button`).
    pub selector: String,
    pub max_width_px: f32,
    pub max_lines: Option<u32>,
    /// `marker`: mapped by the dev key marker (`GLOT_DEV_KEYS=1`, exact); `text`: by rendered text
    /// (two keys with the same source text are ambiguous: the first one gets both boxes).
    pub by: String,
}

pub type Layout = BTreeMap<Key, Fit>;

/// Missing file => empty. Registers the project's `.glot/fonts/` for measuring.
pub fn load_layout(root: &Path) -> Result<Layout> {
    register_fonts_dir(&root.join(FONTS_DIR));
    let path = root.join(LAYOUT_FILE);
    match crate::config::read_optional(&path)? {
        Some(text) => serde_json::from_str(&text).map_err(|e| Error::Parse {
            path,
            message: e.to_string(),
        }),
        None => Ok(Layout::new()),
    }
}

pub fn save_layout(root: &Path, layout: &Layout) -> Result<()> {
    let mut text =
        serde_json::to_string_pretty(layout).map_err(|e| Error::Invalid(e.to_string()))?;
    text.push('\n');
    let path = root.join(LAYOUT_FILE);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    crate::config::write_file(&path, &text)
}

// ---------------------------------------------------------------- font database

struct FaceInfo {
    path: PathBuf,
    index: u32,
    postscript: String,
    families: Vec<String>,
}

#[derive(Default)]
struct Db {
    faces: Vec<FaceInfo>,
    scanned: HashSet<PathBuf>,
    indexed: HashSet<PathBuf>,
    /// Project dirs (`.glot/fonts/`): their fonts also serve as fallbacks.
    project: Vec<PathBuf>,
    data: HashMap<PathBuf, &'static [u8]>,
    /// Shaping faces per font spec (parsing CJK fallbacks per call is slow).
    chains: HashMap<String, (std::sync::Arc<Vec<Face<'static>>>, usize)>,
}

fn db() -> &'static Mutex<Db> {
    static DB: OnceLock<Mutex<Db>> = OnceLock::new();
    DB.get_or_init(Default::default)
}

const SYSTEM_DIRS: &[&str] = &[
    "/System/Library/Fonts",
    "/Library/Fonts",
    "~/Library/Fonts",
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "~/.local/share/fonts",
    "C:\\Windows\\Fonts",
];

/// Browser fallbacks for scripts the primary font lacks (CJK, Arabic…), first existing wins per char.
// ponytail: a fixed list per OS instead of fontconfig/CoreText fallback queries; add entries as users hit gaps.
const FALLBACK_FILES: &[&str] = &[
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/System/Library/Fonts/SFArabic.ttf",
    "/System/Library/Fonts/GeezaPro.ttc",
    "/System/Library/Fonts/SFHebrew.ttf",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "C:\\Windows\\Fonts\\YuGothR.ttc",
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\malgun.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
];

/// Font files of a directory become measurable (`.glot/fonts/` of the project).
pub fn register_fonts_dir(dir: &Path) {
    let mut db = db().lock().unwrap_or_else(|e| e.into_inner());
    if !db.project.iter().any(|d| d == dir) {
        db.project.push(dir.to_path_buf());
        db.chains.clear();
    }
    db.scan(dir);
}

impl Db {
    fn scan(&mut self, dir: &Path) {
        if self.scanned.insert(dir.to_path_buf()) {
            self.walk(dir, None);
        }
    }

    /// Index font files under `dir`; with `like`, only files whose name looks like it (fast path:
    /// "SFNS.ttf" for ".SFNS-Regular") instead of parsing hundreds of system fonts.
    fn walk(&mut self, dir: &Path, like: Option<&str>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                self.walk(&p, like);
                continue;
            }
            let stem = squash(&p.file_stem().unwrap_or_default().to_string_lossy());
            if like.is_none_or(|l| !stem.is_empty() && (l.contains(&stem) || stem.contains(l)))
                && self.indexed.insert(p.clone())
            {
                self.index(&p);
            }
        }
    }

    fn index(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        // ponytail: skips giant files (emoji, full CJK collections) when indexing names; they still work as FALLBACK_FILES.
        let small = std::fs::metadata(path).is_ok_and(|m| m.len() < 40_000_000);
        if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") || !small {
            return;
        }
        let Ok(data) = std::fs::read(path) else {
            return;
        };
        let n = rustybuzz::ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for index in 0..n {
            let Ok(face) = rustybuzz::ttf_parser::Face::parse(&data, index) else {
                continue;
            };
            let name = |id: u16| {
                face.names()
                    .into_iter()
                    .filter(|n| n.name_id == id)
                    .find_map(|n| n.to_string())
            };
            self.faces.push(FaceInfo {
                path: path.to_path_buf(),
                index,
                postscript: name(6).unwrap_or_default(),
                families: [name(16), name(1)].into_iter().flatten().collect(),
            });
        }
    }

    fn bytes(&mut self, path: &Path) -> Option<&'static [u8]> {
        if let Some(d) = self.data.get(path) {
            return Some(d);
        }
        // Fonts in use stay loaded for the process lifetime (a handful of files).
        let d: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
        self.data.insert(path.to_path_buf(), d);
        Some(d)
    }

    fn find(&self, font: &Font) -> Vec<(PathBuf, u32)> {
        let by = |f: &dyn Fn(&FaceInfo) -> bool| -> Vec<(PathBuf, u32)> {
            self.faces
                .iter()
                .filter(|i| f(i))
                .map(|i| (i.path.clone(), i.index))
                .collect()
        };
        if let Some(ps) = font.postscript.as_deref().filter(|p| !p.is_empty()) {
            let hit = by(&|i| i.postscript == ps);
            if !hit.is_empty() {
                return hit;
            }
        }
        by(&|i| {
            i.families
                .iter()
                .any(|f| f.eq_ignore_ascii_case(&font.family))
        })
    }

    /// Primary faces (all files matching the font, e.g. latin + latin-ext subsets), then fallbacks.
    fn chain(&mut self, font: &Font) -> (std::sync::Arc<Vec<Face<'static>>>, usize) {
        let id = format!(
            "{:?}|{}|{}|{}",
            font.postscript, font.family, font.weight, font.size_px
        );
        if let Some(c) = self.chains.get(&id) {
            return c.clone();
        }
        let (faces, primary) = self.load_chain(font);
        let c = (std::sync::Arc::new(faces), primary);
        self.chains.insert(id, c.clone());
        c
    }

    fn load_chain(&mut self, font: &Font) -> (Vec<Face<'static>>, usize) {
        let mut refs = self.find(font);
        if refs.is_empty() {
            for d in SYSTEM_DIRS {
                if !refs.is_empty() {
                    break;
                }
                let d = match d.strip_prefix("~/") {
                    Some(rest) => match std::env::var_os("HOME") {
                        Some(h) => PathBuf::from(h).join(rest),
                        None => continue,
                    },
                    None => PathBuf::from(d),
                };
                for like in [font.postscript.as_deref(), Some(font.family.as_str())]
                    .into_iter()
                    .flatten()
                    .map(squash)
                    .filter(|l| l.len() >= 3)
                {
                    self.walk(&d, Some(&like));
                }
                refs = self.find(font);
                // Full scan only for a font the browser named (a family guess stays on the fast path).
                if refs.is_empty() && font.postscript.is_some() {
                    self.scan(&d);
                    refs = self.find(font);
                }
            }
        }
        let primary = refs.len();
        refs.extend(
            self.faces
                .iter()
                .filter(|i| self.project.iter().any(|d| i.path.starts_with(d)))
                .map(|i| (i.path.clone(), i.index)),
        );
        refs.extend(FALLBACK_FILES.iter().map(|p| (PathBuf::from(p), 0)));
        let mut faces = vec![];
        let mut primary_faces = 0;
        for (k, (path, index)) in refs.into_iter().enumerate() {
            let Some(data) = self.bytes(&path) else {
                continue;
            };
            let Some(mut face) = Face::from_slice(data, index) else {
                continue;
            };
            // Fallbacks take the CSS weight too (variable SF Arabic…); static faces ignore it.
            // ponytail: no bold *file* selection among static fallbacks (Hiragino W3 for every weight).
            face.set_variations(&[
                Variation {
                    tag: Tag::from_bytes(b"wght"),
                    value: font.weight as f32,
                },
                Variation {
                    tag: Tag::from_bytes(b"opsz"),
                    value: font.size_px,
                },
            ]);
            // Apple `trak` tracking is keyed on point size (CSS px == pt for this purpose).
            face.set_points_per_em(Some(font.size_px));
            faces.push(face);
            primary_faces += (k < primary) as usize;
        }
        (faces, primary_faces)
    }
}

/// "Inter-Medium" / ".SFNS-Regular" -> "intermedium" / "sfnsregular".
fn squash(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

// ---------------------------------------------------------------- measuring

/// Average advance (em) per script when no font file covers a char, with a safety margin.
fn fallback_em(c: char) -> f32 {
    let em = match c as u32 {
        0x20 => 0.28,
        0x30..=0x39 => 0.6,
        0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFF60 => {
            1.0
        }
        0x0590..=0x08FF | 0xFB1D..=0xFEFF => 0.5,
        0x0900..=0x0E7F => 0.6,
        _ if c.is_uppercase() => 0.68,
        _ => 0.55,
    };
    em * 1.1
}

/// Width in CSS px of `text` on one line (whitespace collapsed like HTML), shaped with the real
/// font files when found, per-char fallback fonts (CJK, Arabic…), else average metrics.
pub fn measure_text(text: &str, font: &Font) -> f32 {
    let mut text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match font.text_transform.as_deref() {
        Some("uppercase") => text = text.to_uppercase(),
        Some("lowercase") => text = text.to_lowercase(),
        Some("capitalize") => {
            text = text
                .split(' ')
                .map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().chain(c).collect())
                        .unwrap_or_default()
                })
                .collect::<Vec<String>>()
                .join(" ")
        }
        _ => {}
    }
    if text.is_empty() {
        return 0.0;
    }
    let (faces, primary) = db().lock().unwrap_or_else(|e| e.into_inner()).chain(font);
    // Fallback fonts only for other scripts: Latin in an unknown font uses average metrics.
    let pick = |c: char| {
        let n = if (c as u32) > 0x24F && !matches!(c as u32, 0x2000..=0x206F) {
            faces.len()
        } else {
            primary
        };
        faces[..n]
            .iter()
            .position(|f| f.glyph_index(c).is_some())
            .or_else(|| (c.is_whitespace() && primary > 0).then_some(0))
    };
    // Runs of chars rendered by the same face, shaped together (kerning, ligatures, Arabic joining).
    let mut width = 0.0;
    let mut run = String::new();
    let mut cur: Option<Option<usize>> = None;
    let flush = |run: &mut String, face: Option<usize>| -> f32 {
        let w = match face {
            Some(i) => {
                let f = &faces[i];
                let mut buf = UnicodeBuffer::new();
                buf.push_str(run);
                let shaped = rustybuzz::shape(f, features(), buf);
                let units: i32 = shaped.glyph_positions().iter().map(|p| p.x_advance).sum();
                units as f32 * font.size_px / f.units_per_em() as f32
            }
            None => run.chars().map(fallback_em).sum::<f32>() * font.size_px,
        };
        run.clear();
        w
    };
    for c in text.chars() {
        let f = pick(c);
        if cur.is_some_and(|x| x != f) {
            width += flush(&mut run, cur.flatten());
        }
        cur = Some(f);
        run.push(c);
    }
    width += flush(&mut run, cur.flatten());
    width + font.letter_spacing_px * text.chars().count() as f32
}

/// OpenType features browsers enable by default for horizontal text (CSS `font-kerning: auto`,
/// `font-variant-ligatures: normal`), passed explicitly so shaping never depends on shaper defaults.
fn features() -> &'static [rustybuzz::Feature] {
    static F: OnceLock<Vec<rustybuzz::Feature>> = OnceLock::new();
    F.get_or_init(|| {
        [b"kern", b"liga", b"clig", b"calt"]
            .iter()
            .map(|t| rustybuzz::Feature::new(Tag::from_bytes(t), 1, ..))
            .collect()
    })
}

/// Browser width / our width on the source as rendered (1.0 without a reference), clamped.
fn calibration(fit: &Fit) -> f32 {
    match (&fit.text, fit.text_px) {
        (Some(t), Some(px)) if px > 0.0 => {
            let ours = measure_text(t, &fit.font);
            if ours > 0.0 {
                (px / ours).clamp(0.8, 1.25)
            } else {
                1.0
            }
        }
        _ => 1.0,
    }
}

/// Is a char a line-break opportunity on its own (CJK ideographs/kana wrap anywhere)?
fn breaks_anywhere(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Japanese/Chinese kinsoku (minimal, CSS `line-break: normal` like Chromium): no line may start with
/// these closing marks (small kana and ー may, as in the browser)…
fn no_start(c: char) -> bool {
    matches!(
        c,
        '、' | '。'
            | '，'
            | '．'
            | '」'
            | '』'
            | '）'
            | '】'
            | '〉'
            | '》'
            | '〕'
            | '！'
            | '？'
            | '・'
            | '…'
            | '!'
            | '?'
            | ','
            | '.'
            | ')'
            | ':'
            | ';'
    )
}

/// …and none may end with these opening ones.
fn no_end(c: char) -> bool {
    matches!(c, '「' | '『' | '（' | '【' | '〈' | '《' | '〔' | '(')
}

/// Unbreakable units of a whitespace-free word: CJK breaks between any two chars, except
/// before a closing mark or after an opening one (kinsoku); Latin runs inside stay whole.
fn break_units(w: &str) -> Vec<String> {
    if !w.chars().any(breaks_anywhere) {
        return vec![w.to_string()];
    }
    let mut units: Vec<String> = vec![];
    let mut prev: Option<char> = None;
    for c in w.chars() {
        let split = prev.is_some_and(|p| {
            (breaks_anywhere(c) || breaks_anywhere(p)) && !no_start(c) && !no_end(p)
        });
        match units.last_mut() {
            Some(u) if !split => u.push(c),
            _ => units.push(c.to_string()),
        }
        prev = Some(c);
    }
    units
}

/// Lines needed to lay out `text` greedily in `width` px (`u32::MAX` if a single word is wider).
/// `words`: (separated from the previous one by a space, width). CJK units of one word have no space.
fn lines_needed(words: &[(bool, f32)], space: f32, width: f32) -> u32 {
    let mut lines = 1;
    let mut x = 0.0;
    for (i, (gap, w)) in words.iter().enumerate() {
        if *w > width + 0.5 {
            return u32::MAX;
        }
        let add = if i == 0 || x == 0.0 || !gap {
            *w
        } else {
            space + w
        };
        if x + add > width + 0.5 {
            lines += 1;
            x = *w;
        } else {
            x += add;
        }
    }
    lines
}

/// `(width needed on one line, lines needed in the box)` for a plain rendered string.
fn layout_of(text: &str, fit: &Fit, k: f32) -> (f32, u32) {
    let one = measure_text(text, &fit.font) * k;
    let mut words: Vec<(bool, f32)> = vec![];
    for w in text.split_whitespace() {
        // ponytail: minimal kinsoku (closing/opening marks); no hanging punctuation or line-end compression.
        for (i, u) in break_units(w).iter().enumerate() {
            words.push((i == 0, measure_text(u, &fit.font) * k));
        }
    }
    let space = measure_text("a a", &fit.font) * k - measure_text("aa", &fit.font) * k;
    (one, lines_needed(&words, space.max(0.0), fit.max_width_px))
}

/// How many average Latin characters fit in the budget (the prompt hint).
pub fn approx_chars(fit: &Fit) -> usize {
    let sample = "the quick brown fox jumps over the lazy dog";
    let avg = measure_text(sample, &fit.font) / sample.chars().count() as f32;
    let lines = fit.max_lines.unwrap_or(1).max(1) as f32;
    if avg <= 0.0 {
        return 0;
    }
    (fit.max_width_px * lines / avg).floor() as usize
}

/// "must fit in 96px at Inter 14px/500 ≈ 12 Latin chars" — fed to the translation prompt.
pub fn hint(fit: &Fit) -> String {
    let f = &fit.font;
    let font = format!("{} {}px/{}", f.family, f.size_px.round(), f.weight);
    let n = approx_chars(fit);
    match fit.max_lines {
        Some(1) => format!(
            "must fit on one line in {}px at {font} ≈ {n} Latin chars",
            fit.max_width_px.round()
        ),
        Some(l) => format!(
            "must fit in {l} lines of {}px at {font} ≈ {n} Latin chars in total",
            fit.max_width_px.round()
        ),
        None => format!(
            "lines wrap at {}px at {font}: no single word may be wider (≈ {n} Latin chars)",
            fit.max_width_px.round()
        ),
    }
}

/// `TooWide` for a message that does not fit its measured box: error over budget, warning within
/// `MARGIN` of it. Placeholders are rendered with sample values, plurals with their longest branch.
pub fn check(target: &str, syntax: Syntax, fit: &Fit) -> Option<Issue> {
    let text = render_sample(target, syntax);
    if text.trim().is_empty() || fit.max_width_px <= 0.0 {
        return None;
    }
    let k = calibration(fit);
    let (one, _) = layout_of(&text, fit, k);
    let allowed = fit.max_lines.unwrap_or(u32::MAX);
    let fits = |w: f32| {
        let f = Fit {
            max_width_px: w,
            ..fit.clone()
        };
        let lines = layout_of(&text, &f, k).1;
        lines != u32::MAX && lines <= allowed
    };
    let budget = fit.max_width_px;
    let n = approx_chars(fit);
    let font = format!(
        "{} {}px/{}",
        fit.font.family,
        fit.font.size_px.round(),
        fit.font.weight
    );
    let room = match fit.max_lines {
        Some(1) => format!("one line of {}px", budget.round()),
        Some(l) => format!("{l} lines of {}px", budget.round()),
        None => format!(
            "lines of {}px (a single word may not be wider)",
            budget.round()
        ),
    };
    if !fits(budget) {
        let over = match fit.max_lines {
            Some(1) => format!(
                " it renders {}px wide, {}% too wide;",
                one.round(),
                ((one / budget - 1.0) * 100.0).ceil()
            ),
            _ => String::new(),
        };
        Some(Issue {
            code: IssueCode::TooWide,
            severity: Severity::Error,
            message: format!(
                "Too wide for its box in the app: the text must fit in {room} at {font};{over} use a shorter wording (≈ {n} Latin chars max), never truncate or add an ellipsis."
            ),
        })
    } else if !fits(budget * (1.0 - MARGIN)) {
        Some(Issue {
            code: IssueCode::TooWide,
            severity: Severity::Warning,
            message: format!(
                "Barely fits its box ({room} at {font}, within {}%): a slightly shorter wording is safer.",
                (MARGIN * 100.0) as u32
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fit(w: f32, lines: Option<u32>) -> Fit {
        Fit {
            max_width_px: w,
            max_lines: lines,
            font: Font {
                family: "glot-test-no-such-font".into(),
                postscript: None,
                size_px: 10.0,
                weight: 400,
                letter_spacing_px: 0.0,
                text_transform: None,
            },
            text: None,
            text_px: None,
            occurrences: vec![],
        }
    }

    #[test]
    fn fallback_metrics_scale_and_wrap() {
        let f = fit(100.0, Some(1)).font;
        let w = measure_text("abc", &f);
        assert!((w - 3.0 * 0.55 * 1.1 * 10.0).abs() < 0.01, "{w}");
        assert!(measure_text("日本", &f) > measure_text("ab", &f) * 1.5);
        assert_eq!(measure_text("  a   b ", &f), measure_text("a b", &f));
        let ls = Font {
            letter_spacing_px: 1.0,
            ..f.clone()
        };
        assert!((measure_text("abc", &ls) - w - 3.0).abs() < 0.01);
        let up = Font {
            text_transform: Some("uppercase".into()),
            ..f.clone()
        };
        assert!(measure_text("abc", &up) > w);
        // 6.05px per char: 16 chars ≈ 97px.
        assert!(check("Short", Syntax::Icu, &fit(100.0, Some(1))).is_none());
        let e = check("A much longer label", Syntax::Icu, &fit(100.0, Some(1))).unwrap();
        assert_eq!((e.code, e.severity), (IssueCode::TooWide, Severity::Error));
        assert!(e.message.contains("100px"), "{}", e.message);
        // Two lines of 100px hold it; unlimited lines only need each word to fit.
        assert!(check("A much longer label", Syntax::Icu, &fit(100.0, Some(2))).is_none());
        assert!(
            check(
                "Supercalifragilisticexpialidocious",
                Syntax::Icu,
                &fit(100.0, None)
            )
            .is_some()
        );
        // Within 5 %: warning.
        let w = check("abcdefghijklmno", Syntax::Icu, &fit(92.0, Some(1))).unwrap();
        assert_eq!(w.severity, Severity::Warning);
        // Placeholders render with sample values; plurals by their longest branch.
        assert!(
            check(
                "{count, plural, one {# file} other {# files in the folder}}",
                Syntax::Icu,
                &fit(60.0, Some(1))
            )
            .is_some()
        );
    }

    #[test]
    fn kinsoku_units() {
        assert_eq!(break_units("保存する。"), ["保", "存", "す", "る。"]);
        assert_eq!(
            break_units("「設定」を開く、"),
            ["「設", "定」", "を", "開", "く、"]
        );
        assert_eq!(break_units("Glotで翻訳"), ["Glot", "で", "翻", "訳"]);
        assert_eq!(break_units("ちょっと"), ["ち", "ょ", "っ", "と"]);
        assert_eq!(break_units("hello"), ["hello"]);
    }

    /// Line count vs Chromium on Japanese with punctuation (fixture: `packages/visual-qa/test/measure.test.ts`, macOS only).
    #[test]
    fn kinsoku_matches_browser_lines() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/browser-lines-ja.json");
        if !cfg!(target_os = "macos") || !file.exists() {
            return;
        }
        let fixture: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        let (mut bad, mut off) = (vec![], 0);
        let cases = fixture["cases"].as_array().unwrap();
        for case in cases {
            let font: Font = serde_json::from_value(case["font"].clone()).unwrap();
            let f = Fit {
                max_width_px: case["width"].as_f64().unwrap() as f32,
                max_lines: None,
                font,
                text: None,
                text_px: None,
                occurrences: vec![],
            };
            let text = case["text"].as_str().unwrap();
            let ours = layout_of(text, &f, 1.0).1;
            let browser = case["lines"].as_u64().unwrap() as u32;
            // Never optimistic; one extra line at most (Chromium's `text-spacing-trim` half-width
            // punctuation at line ends is not modelled).
            if ours < browser || ours > browser + 1 {
                bad.push(format!(
                    "{text:?} @{}px: rust {ours} vs browser {browser}",
                    f.max_width_px
                ));
            }
            off += (ours != browser) as usize;
        }
        eprintln!(
            "kinsoku: {off}/{} line counts differ from Chromium",
            cases.len()
        );
        assert!(bad.is_empty(), "{bad:#?}");
        assert!(off * 20 <= cases.len(), "{off} mismatches");
    }

    #[test]
    fn calibration_uses_browser_reference() {
        let mut f = fit(100.0, Some(1));
        let ours = measure_text("Hello", &f.font);
        f.text = Some("Hello".into());
        f.text_px = Some(ours * 1.2);
        assert!((calibration(&f) - 1.2).abs() < 0.001);
        f.text_px = Some(ours * 3.0);
        assert_eq!(calibration(&f), 1.25);
    }

    /// Rust shaping vs Chromium on 30 strings (fixture made by `packages/visual-qa/test/measure.test.ts`).
    #[test]
    fn matches_browser_within_5_percent() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        // GLOT_MEASURE_FIXTURE: same shape, made on this machine (system fonts, CJK/Arabic fallbacks).
        let file = std::env::var("GLOT_MEASURE_FIXTURE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dir.join("browser-widths.json"));
        let fixture: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        register_fonts_dir(&dir);
        let mut worst: f32 = 0.0;
        for case in fixture["cases"].as_array().unwrap() {
            let font: Font = serde_json::from_value(case["font"].clone()).unwrap();
            let text = case["text"].as_str().unwrap();
            let browser = case["px"].as_f64().unwrap() as f32;
            let ours = measure_text(text, &font);
            let err = (ours - browser).abs() / browser;
            worst = worst.max(err);
            if err >= 0.05 {
                eprintln!("{text:?} {font:?}: rust {ours:.1}px vs browser {browser:.1}px");
            }
        }
        eprintln!("worst relative error: {:.2}%", worst * 100.0);
        assert!(fixture["cases"].as_array().unwrap().len() >= 30);
        assert!(worst < 0.05);
    }
}
