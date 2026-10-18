//! Glyph coverage: which characters of each locale a webfont has no glyph for (its `cmap`), so
//! the browser/OS draws them with a fallback font (another look, other metrics) or as tofu.
//!
//! Reads `.ttf`/`.otf`/`.ttc` and `.woff2` (Brotli stream, `cmap`/`name` are never transformed so
//! no glyph reconstruction is needed).
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{Seg, segments};
use crate::validate::Syntax;
use rustybuzz::ttf_parser::{self, Tag, cmap, name};

#[derive(Debug, Clone, Serialize)]
pub struct Family {
    pub family: String,
    pub files: Vec<PathBuf>,
    /// `glot.toml` (`[measure] fonts`), `--font`, or `detected`.
    pub origin: String,
    pub glyphs: usize,
    #[serde(skip)]
    pub cps: HashSet<u32>,
}

/// One font × locale: what falls back.
#[derive(Debug, Clone, Serialize)]
pub struct Gap {
    pub family: String,
    pub locale: String,
    pub keys: usize,
    pub key_sample: Vec<String>,
    /// Distinct characters without a glyph, in code point order.
    pub missing: String,
    pub scripts: Vec<String>,
    /// What the platform will most likely draw them with.
    pub fallback: String,
    /// `unicode-range` of the subset to add (whole blocks, like Google Fonts slices).
    pub unicode_range: String,
    pub estimated_kb: u32,
}

/// Raw `cmap` and `name` tables of one face.
type Tables = (Option<Vec<u8>>, Option<Vec<u8>>);

/// (family, code points with a glyph) of every face in a font file.
pub fn read_faces(data: &[u8]) -> Vec<(String, HashSet<u32>)> {
    let tables: Vec<Tables> = if data.starts_with(b"wOF2") {
        woff2_tables(data).into_iter().collect()
    } else {
        let n = ttf_parser::fonts_in_collection(data).unwrap_or(1);
        (0..n)
            .filter_map(|i| ttf_parser::RawFace::parse(data, i).ok())
            .map(|f| {
                let t = |tag: &[u8; 4]| f.table(Tag::from_bytes(tag)).map(<[u8]>::to_vec);
                (t(b"cmap"), t(b"name"))
            })
            .collect()
    };
    tables
        .into_iter()
        .filter_map(|(c, n)| {
            let cmap = cmap::Table::parse(c.as_deref()?)?;
            let mut cps = HashSet::new();
            for st in cmap.subtables {
                if st.is_unicode() {
                    st.codepoints(|cp| {
                        if st.glyph_index(cp).is_some_and(|g| g.0 != 0) {
                            cps.insert(cp);
                        }
                    });
                }
            }
            let family = n
                .as_deref()
                .and_then(name::Table::parse)
                .and_then(|t| {
                    [16u16, 1].iter().find_map(|id| {
                        t.names
                            .into_iter()
                            .filter(|r| r.name_id == *id && r.is_unicode())
                            .find_map(|r| r.to_string())
                    })
                })
                .unwrap_or_default();
            Some((family, cps))
        })
        .collect()
}

/// `cmap` and `name` of a (non-collection) WOFF2 file.
// ponytail: WOFF2 collections (`ttcf` flavor) and WOFF 1 are skipped; both are rare on the web.
fn woff2_tables(d: &[u8]) -> Option<Tables> {
    let u16_at = |i: usize| Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?));
    if d.get(4..8)? == b"ttcf" {
        return None;
    }
    let num = u16_at(12)? as usize;
    let compressed = u32_at(20)? as usize;
    let mut pos = 48;
    let base128 = |pos: &mut usize| -> Option<u32> {
        let mut v: u32 = 0;
        for _ in 0..5 {
            let b = *d.get(*pos)?;
            *pos += 1;
            v = v.checked_mul(128)? | (b & 0x7f) as u32;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    };
    // (known-tag index, offset in the decompressed stream, length)
    let mut dir = vec![];
    let mut offset = 0usize;
    for _ in 0..num {
        let flags = *d.get(pos)?;
        pos += 1;
        let idx = flags & 0x3f;
        if idx == 63 {
            pos += 4;
        }
        let orig = base128(&mut pos)? as usize;
        let version = flags >> 6;
        // glyf (10) / loca (11) are transformed by version 0; every other table by a non-zero one.
        let transformed = if idx == 10 || idx == 11 {
            version == 0
        } else {
            version != 0
        };
        let len = if transformed {
            base128(&mut pos)? as usize
        } else {
            orig
        };
        dir.push((idx, offset, len));
        offset += len;
    }
    let mut stream = vec![];
    brotli_decompressor::Decompressor::new(d.get(pos..pos + compressed)?, 4096)
        .take(64 << 20)
        .read_to_end(&mut stream)
        .ok()?;
    let table = |want: u8| {
        dir.iter()
            .find(|t| t.0 == want)
            .and_then(|t| stream.get(t.1..t.1 + t.2))
            .map(<[u8]>::to_vec)
    };
    Some((table(0), table(5)))
}

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "Pods",
    "DerivedData",
    "vendor",
    "cache",
    ".git",
];

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if p.is_dir() {
            let hidden = name.starts_with('.') && name != ".next" && name != ".glot";
            if depth < 12 && !hidden && !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, depth + 1, out);
            }
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str())
            && matches!(
                ext.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "woff2" | "woff"
            )
        {
            out.push(p);
        }
    }
}

/// Fonts to check: `declared` entries (file, glob, or a family name to keep among the detected
/// ones), else every font file of the repo (`public/fonts`, `.glot/fonts`, `.next/static/media`
/// for next/font). Icon fonts (no a-z) are left out. Returns notes for the report.
pub fn discover(root: &Path, declared: &[String], origin: &str) -> (Vec<Family>, Vec<String>) {
    let mut notes = vec![];
    let mut files: Vec<(PathBuf, String)> = vec![];
    let mut keep_families: Vec<String> = vec![];
    for d in declared {
        let p = if Path::new(d).is_absolute() {
            PathBuf::from(d)
        } else {
            root.join(d)
        };
        if d.contains(['*', '?', '[']) {
            let hits: Vec<PathBuf> = glob::glob(&p.to_string_lossy())
                .map(|g| g.flatten().collect())
                .unwrap_or_default();
            if hits.is_empty() {
                notes.push(format!("{d}: no file matches"));
            }
            files.extend(hits.into_iter().map(|h| (h, origin.to_string())));
        } else if p.is_file() {
            files.push((p, origin.to_string()));
        } else {
            keep_families.push(d.to_lowercase());
        }
    }
    if files.is_empty() {
        let mut found = vec![];
        walk(root, 0, &mut found);
        files.extend(found.into_iter().map(|f| (f, "detected".to_string())));
    }
    let mut by: BTreeMap<String, Family> = BTreeMap::new();
    let mut woff1 = 0;
    for (path, origin) in files {
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        if data.starts_with(b"wOFF") {
            woff1 += 1;
            continue;
        }
        for (family, cps) in read_faces(&data) {
            if !(b'a'..=b'z').all(|c| cps.contains(&(c as u32))) {
                continue; // icon / symbol font
            }
            let family = if family.is_empty() {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            } else {
                family
            };
            let f = by.entry(family.to_lowercase()).or_insert_with(|| Family {
                family: family.clone(),
                files: vec![],
                origin: origin.clone(),
                glyphs: 0,
                cps: HashSet::new(),
            });
            if !f.files.contains(&path) {
                f.files
                    .push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
            f.cps.extend(cps);
            f.glyphs = f.cps.len();
        }
    }
    if woff1 > 0 {
        notes.push(format!(
            "{woff1} WOFF 1 file(s) skipped (convert to woff2 or declare the .ttf)"
        ));
    }
    let mut out: Vec<Family> = by.into_values().collect();
    if !keep_families.is_empty() {
        let missing: Vec<&String> = keep_families
            .iter()
            .filter(|k| !out.iter().any(|f| f.family.to_lowercase() == **k))
            .collect();
        for m in missing {
            notes.push(format!(
                "font \"{m}\": no file found in the repo (declare its path)"
            ));
        }
        out.retain(|f| keep_families.contains(&f.family.to_lowercase()));
    }
    if out.is_empty() && declared.is_empty() {
        notes.push(next_font_hint(root).unwrap_or_else(|| {
            "no webfont in the repo: text renders with platform fonts (full coverage); declare [measure] fonts = [\"path/to/font.woff2\"] in glot.toml to check one".into()
        }));
    }
    (out, notes)
}

/// next/font/google fonts live in `.next/static/media` only after a build.
fn next_font_hint(root: &Path) -> Option<String> {
    let dirs = ["app", "src/app", "pages", "src/pages"];
    for d in dirs {
        for f in [
            "layout.tsx",
            "layout.jsx",
            "_app.tsx",
            "_app.jsx",
            "layout.ts",
            "layout.js",
        ] {
            let t = std::fs::read_to_string(root.join(d).join(f)).unwrap_or_default();
            if t.contains("next/font/google") {
                return Some("next/font/google detected but no font file yet: run `next build` (fonts land in .next/static/media) or declare [measure] fonts".into());
            }
        }
    }
    None
}

/// Unicode block → script name, platform fallback, ≈ bytes per glyph in a woff2 subset.
// ponytail: a hand table of the scripts apps ship in, not the full Scripts.txt; the size is an
// order of magnitude (complex scripts also need their GSUB/GPOS).
const CJK: &str = "Hiragino Sans / PingFang / Noto Sans CJK / Yu Gothic / Microsoft YaHei";
const SCRIPTS: &[(u32, u32, &str, &str, u32)] = &[
    (0x0000, 0x024F, "Latin", "system UI font", 80),
    (0x0300, 0x036F, "Combining marks", "system UI font", 80),
    (0x0370, 0x03FF, "Greek", "system UI font", 80),
    (0x0400, 0x052F, "Cyrillic", "system UI font", 90),
    (
        0x0590,
        0x05FF,
        "Hebrew",
        "SF Hebrew / Arial Hebrew / Noto Sans Hebrew",
        120,
    ),
    (
        0x0600,
        0x06FF,
        "Arabic",
        "Geeza Pro / SF Arabic / Noto Naskh Arabic / Segoe UI",
        300,
    ),
    (
        0x0750,
        0x077F,
        "Arabic",
        "Geeza Pro / SF Arabic / Noto Naskh Arabic / Segoe UI",
        300,
    ),
    (
        0x0900,
        0x097F,
        "Devanagari",
        "Kohinoor Devanagari / Noto Sans Devanagari / Nirmala UI",
        350,
    ),
    (
        0x0980,
        0x09FF,
        "Bengali",
        "Kohinoor Bangla / Noto Sans Bengali / Nirmala UI",
        350,
    ),
    (
        0x0B80,
        0x0BFF,
        "Tamil",
        "Tamil Sangam MN / Noto Sans Tamil / Nirmala UI",
        300,
    ),
    (
        0x0E00,
        0x0E7F,
        "Thai",
        "Thonburi / Noto Sans Thai / Leelawadee UI",
        150,
    ),
    (0x10A0, 0x10FF, "Georgian", "system UI font", 100),
    (
        0x1100,
        0x11FF,
        "Hangul",
        "Apple SD Gothic Neo / Noto Sans KR / Malgun Gothic",
        250,
    ),
    (
        0x1E00,
        0x1EFF,
        "Latin Extended (Vietnamese)",
        "system UI font",
        80,
    ),
    (0x2000, 0x206F, "Punctuation", "system UI font", 60),
    (0x20A0, 0x20CF, "Currency symbols", "system UI font", 80),
    (
        0x2100,
        0x2BFF,
        "Symbols & arrows",
        "system UI / symbol font",
        120,
    ),
    (0x3000, 0x303F, "CJK punctuation", CJK, 200),
    (0x3040, 0x30FF, "Kana", CJK, 250),
    (
        0x3130,
        0x318F,
        "Hangul",
        "Apple SD Gothic Neo / Noto Sans KR / Malgun Gothic",
        250,
    ),
    (0x3400, 0x4DBF, "Han", CJK, 400),
    (0x4E00, 0x9FFF, "Han", CJK, 400),
    (
        0xAC00,
        0xD7AF,
        "Hangul",
        "Apple SD Gothic Neo / Noto Sans KR / Malgun Gothic",
        250,
    ),
    (
        0xFB50,
        0xFDFF,
        "Arabic",
        "Geeza Pro / SF Arabic / Noto Naskh Arabic / Segoe UI",
        300,
    ),
    (
        0xFE70,
        0xFEFF,
        "Arabic",
        "Geeza Pro / SF Arabic / Noto Naskh Arabic / Segoe UI",
        300,
    ),
    (0xFF00, 0xFFEF, "Fullwidth forms", CJK, 200),
];

/// Drawn by the OS emoji font whatever the webfont (or not drawn at all: controls, joiners).
fn ignored(c: char) -> bool {
    let u = c as u32;
    c.is_whitespace()
        || c.is_control()
        || matches!(u, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF)
        || matches!(u, 0x2600..=0x27BF | 0x1F000..=0x1FAFF | 0xE0000..=0xE007F)
}

/// Characters each locale needs that a family has no glyph for, worst first.
pub fn gaps<'a>(
    families: &[Family],
    catalogs: impl IntoIterator<Item = (&'a String, &'a BTreeMap<String, String>)>,
    syntax: Syntax,
) -> Vec<Gap> {
    let catalogs: Vec<_> = catalogs.into_iter().collect();
    let mut out = vec![];
    for f in families {
        for (locale, entries) in &catalogs {
            let mut keys = vec![];
            let mut missing: BTreeSet<char> = BTreeSet::new();
            for (key, text) in *entries {
                let segs = segments(text, syntax);
                let miss: Vec<char> = segs
                    .iter()
                    .filter(|s| s.seg == Seg::Text)
                    .flat_map(|s| text[s.range.clone()].chars())
                    .filter(|c| !ignored(*c) && !f.cps.contains(&(*c as u32)))
                    .collect();
                if !miss.is_empty() {
                    keys.push(key.clone());
                    missing.extend(miss);
                }
            }
            if keys.is_empty() {
                continue;
            }
            let mut scripts: Vec<String> = vec![];
            let mut fallbacks: Vec<&str> = vec![];
            let mut blocks: BTreeSet<(u32, u32)> = BTreeSet::new();
            let mut bytes = 0u32;
            for c in &missing {
                let u = *c as u32;
                let s = SCRIPTS.iter().find(|s| (s.0..=s.1).contains(&u));
                let (name, fb, per) =
                    s.map_or(("Other", "system UI font", 100), |s| (s.2, s.3, s.4));
                blocks.insert(s.map_or((u, u), |s| (s.0, s.1)));
                bytes += per;
                if !scripts.iter().any(|x| x == name) {
                    scripts.push(name.into());
                }
                if !fallbacks.contains(&fb) {
                    fallbacks.push(fb);
                }
            }
            let unicode_range = blocks
                .iter()
                .map(|(a, b)| {
                    if a == b {
                        format!("U+{a:04X}")
                    } else {
                        format!("U+{a:04X}-{b:04X}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            out.push(Gap {
                family: f.family.clone(),
                locale: (*locale).clone(),
                keys: keys.len(),
                key_sample: keys.into_iter().take(5).collect(),
                missing: missing.into_iter().collect(),
                scripts,
                fallback: fallbacks.join(" + "),
                unicode_range,
                estimated_kb: bytes.div_ceil(1024),
            });
        }
    }
    out.sort_by(|a, b| b.keys.cmp(&a.keys).then(a.locale.cmp(&b.locale)));
    out
}
