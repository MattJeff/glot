//! Images with text (`glot assets --svg --og`).
//!
//! - **SVG** (`assets/*.svg`): copy of `<text>`/`<tspan>`/`<title>` becomes keys `svg.<name>.<id|slug>`;
//!   the file is never rewritten, translations are substituted per locale ([`localize`]). Each `<text>`
//!   gets a pixel budget ([`Fit`], one line): `data-max-width`, else the room left in the viewBox from
//!   `x` given `text-anchor`. The budget goes to `.glot/layout.json`, so `glot_core::measure` rejects a
//!   too-wide translation (`too_wide`) and the translate pipeline retries it before anything is written.
//! - **OG images in JSX** (`og/*.tsx`, the `@vercel/og`/satori subset): evaluated like an email
//!   ([`crate::email::render_jsx`]) and laid out by [`og_svg`] into an SVG (flex column of text blocks,
//!   wrapped with the same measure), rendered to PNG by the CLI (resvg). Blocks rendered by one `t()` get
//!   a budget: box width × `lineClamp` lines.
use glot_core::measure::{Fit, Font, measure_text};

use crate::email::{Node, Seg, Tag, is_rtl, name_segments, scan_markup};

/// One piece of copy of an SVG, with its key and (for `<text>`) its pixel budget.
#[derive(Debug, Clone)]
pub struct SvgText {
    pub key: String,
    pub seg: Seg,
    pub fit: Option<Fit>,
    /// `<text>` element it belongs to (its `<` offset), to find its rendered box.
    pub text_at: Option<usize>,
}

/// `assets/hero-banner.svg` -> `hero_banner`.
pub fn name(path: &str) -> String {
    crate::email::template_name(path)
}

/// Inherited presentation attribute (own attribute or `style="…"`, innermost first).
fn inherited(stack: &[Tag], name: &str) -> Option<String> {
    stack.iter().rev().find_map(|t| {
        t.attr(name).map(str::to_string).or_else(|| {
            t.attr("style")?.split(';').find_map(|d| {
                let (k, v) = d.split_once(':')?;
                (k.trim() == name).then(|| v.trim().to_string())
            })
        })
    })
}

fn px(v: &str) -> Option<f32> {
    v.trim().trim_end_matches("px").trim().parse().ok()
}

fn weight(v: Option<String>) -> u16 {
    match v.as_deref() {
        Some("bold" | "bolder") => 700,
        Some(w) => w.parse().unwrap_or(400),
        None => 400,
    }
}

/// First family of a CSS font-family list, unquoted.
pub fn family(v: Option<&str>) -> String {
    v.and_then(|f| f.split(',').next())
        .map(|f| f.trim().trim_matches(['"', '\'']).to_string())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "Helvetica".into())
}

/// Copy of an SVG document with keys and budgets.
pub fn extract(path: &str, src: &str) -> Vec<SvgText> {
    let keep = |st: &[Tag], attr: Option<&str>| {
        let top = st.last().map(|t| t.name.as_str()).unwrap_or("");
        attr.is_none() && matches!(top, "text" | "tspan" | "title" | "textpath")
    };
    let segs = scan_markup(src, &keep);
    let keys = name_segments(&format!("svg.{}", name(path)), &segs, &|_| None);
    keys.into_iter()
        .zip(segs)
        .map(|(key, seg)| {
            let text = seg.stack.iter().rposition(|t| t.name == "text");
            let fit = text.map(|i| budget(&seg.stack, i));
            let text_at = text.map(|i| seg.stack[i].start);
            SvgText {
                key,
                seg,
                fit,
                text_at,
            }
        })
        .collect()
}

fn budget(stack: &[Tag], text: usize) -> Fit {
    let el = &stack[text];
    let upto = &stack[..=text];
    let root = stack.iter().find(|t| t.name == "svg");
    let view_w = root
        .and_then(|r| {
            r.attr("viewBox")
                .and_then(|v| {
                    v.split([' ', ','])
                        .filter(|x| !x.is_empty())
                        .nth(2)
                        .and_then(px)
                })
                .or_else(|| r.attr("width").and_then(px))
        })
        .unwrap_or(0.0);
    let x = el
        .attr("x")
        .and_then(|v| v.split([' ', ',']).next().and_then(px))
        .unwrap_or(0.0);
    let width = el.attr("data-max-width").and_then(px).unwrap_or_else(|| {
        match inherited(upto, "text-anchor").as_deref() {
            Some("middle") => 2.0 * x.min(view_w - x),
            Some("end") => x,
            _ => view_w - x,
        }
    });
    Fit {
        max_width_px: width.max(0.0),
        max_lines: Some(1),
        font: Font {
            family: family(inherited(upto, "font-family").as_deref()),
            postscript: None,
            size_px: inherited(upto, "font-size")
                .and_then(|v| px(&v))
                .unwrap_or(16.0),
            weight: weight(inherited(upto, "font-weight")),
            letter_spacing_px: inherited(upto, "letter-spacing")
                .and_then(|v| px(&v))
                .unwrap_or(0.0),
            text_transform: None,
        },
        text: None,
        text_px: None,
        occurrences: vec![],
    }
}

/// The SVG with translations substituted; `ids` adds `id="glot-<offset>"` to `<text>` elements
/// without one (in-memory render only: maps rendered boxes back to keys).
pub fn localize(src: &str, texts: &[SvgText], value: &dyn Fn(&str) -> String, ids: bool) -> String {
    let mut repl: Vec<((usize, usize), bool, String)> = texts
        .iter()
        .map(|t| (t.seg.span, t.seg.attr.is_some(), value(&t.key)))
        .collect();
    if ids {
        let mut seen = std::collections::BTreeSet::new();
        for t in texts {
            if let Some(at) = t.text_at
                && seen.insert(at)
                && !t
                    .seg
                    .stack
                    .iter()
                    .any(|s| s.start == at && s.attr("id").is_some())
            {
                // `<text` is 5 bytes: insert right after the tag name (raw text, not escaped).
                repl.push(((at + 5, at + 5), false, format!(" id=\"glot-{at}\"")));
            }
        }
    }
    let mut sorted = repl;
    sorted.sort_by_key(|(s, _, _)| *s);
    let mut out = String::with_capacity(src.len() + 64);
    let mut pos = 0;
    for ((s, e), attr, text) in sorted {
        if s < pos {
            continue;
        }
        out.push_str(&src[pos..s]);
        if s == e {
            out.push_str(&text);
        } else {
            let t = crate::email::from_icu(&text);
            out.push_str(&if attr {
                crate::email::esc_attr(&t)
            } else {
                esc(&t)
            });
        }
        pos = e;
    }
    out.push_str(&src[pos..]);
    out
}

/// Id of the `<text>` element of `t` in the SVG produced by `localize(…, ids = true)`.
pub fn text_id(t: &SvgText) -> Option<String> {
    let at = t.text_at?;
    let own = t.seg.stack.iter().find(|s| s.start == at)?.attr("id");
    Some(own.map_or_else(|| format!("glot-{at}"), str::to_string))
}

// ---------------------------------------------------------------- OG images (JSX -> SVG)

#[derive(Debug, Clone)]
pub struct Og {
    pub svg: String,
    pub width: f32,
    pub height: f32,
    /// Budget of each block rendered by a single `t()` call.
    pub fits: Vec<(String, Fit)>,
    /// Content taller than the image (px over), after wrapping.
    pub overflow_px: f32,
}

fn style<'n>(n: &'n Node, k: &str) -> Option<&'n str> {
    match n {
        Node::El { style, .. } => style
            .iter()
            .rev()
            .find(|(s, _)| s == k)
            .map(|(_, v)| v.as_str()),
        _ => None,
    }
}

fn num(n: &Node, k: &str) -> Option<f32> {
    style(n, k).and_then(px)
}

/// `padding: 80` / `"40px 80px"` (+ `paddingLeft`…) -> (top, right, bottom, left).
fn padding(n: &Node) -> [f32; 4] {
    let mut p = [0.0; 4];
    if let Some(v) = style(n, "padding") {
        let xs: Vec<f32> = v.split_whitespace().filter_map(px).collect();
        p = match xs.as_slice() {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d, ..] => [*a, *b, *c, *d],
            [] => p,
        };
    }
    for (i, k) in ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft"]
        .iter()
        .enumerate()
    {
        if let Some(v) = num(n, k) {
            p[i] = v;
        }
    }
    p
}

/// First CSS color of a value (`linear-gradient(…, #111, …)` -> `#111`).
fn color(v: &str) -> Option<String> {
    let v = v.trim();
    if !v.contains('(') || v.starts_with("rgb") {
        return Some(v.to_string());
    }
    let i = v.find('#')?;
    Some(
        v[i..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '#')
            .collect(),
    )
}

#[derive(Clone)]
struct Inherit {
    family: String,
    size: f32,
    weight: u16,
    color: String,
    line_height: f32,
    align: String,
}

struct Block {
    key: Option<String>,
    text: String,
    width: f32,
    lines_max: Option<u32>,
    margin: (f32, f32),
    st: Inherit,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Greedy wrap with the measuring font (CJK breaks anywhere).
pub fn wrap(text: &str, width: f32, font: &Font) -> Vec<String> {
    let mut tokens: Vec<String> = vec![];
    for w in text.split_whitespace() {
        if w.chars()
            .any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x9FFF | 0xFF00..=0xFFEF))
        {
            tokens.extend(w.chars().map(String::from));
        } else {
            tokens.push(format!(" {w}"));
        }
    }
    let mut lines: Vec<String> = vec![];
    let mut cur = String::new();
    for t in tokens {
        let cand = format!("{cur}{t}");
        if !cur.is_empty() && measure_text(cand.trim(), font) > width {
            lines.push(cur.trim().to_string());
            cur = t.trim_start().to_string();
        } else {
            cur = cand;
        }
    }
    if !cur.trim().is_empty() {
        lines.push(cur.trim().to_string());
    }
    lines
}

/// Lay out an evaluated OG template (`locale` sets the direction) into a standalone SVG.
// ponytail: satori subset = a flex column of text blocks (no rows, images or absolute positioning);
// for richer cards render with @vercel/og and keep `glot check` for the budgets.
pub fn og_svg(root: &Node, locale: &str) -> Og {
    let width = num(root, "width").unwrap_or(1200.0);
    let height = num(root, "height").unwrap_or(630.0);
    let pad = padding(root);
    let rtl = is_rtl(locale);
    let st = Inherit {
        family: "Helvetica".into(),
        size: 16.0,
        weight: 400,
        color: "#000".into(),
        line_height: 1.2,
        align: style(root, "alignItems")
            .filter(|a| *a == "center")
            .map(|_| "center".to_string())
            .unwrap_or_else(|| "start".into()),
    };
    let mut blocks = vec![];
    collect(root, &st, width, &mut blocks);
    let font = |b: &Block| Font {
        family: b.st.family.clone(),
        postscript: None,
        size_px: b.st.size,
        weight: b.st.weight,
        letter_spacing_px: 0.0,
        text_transform: None,
    };
    let laid: Vec<(Vec<String>, f32)> = blocks
        .iter()
        .map(|b| {
            let mut lines = wrap(&b.text, b.width, &font(b));
            if let Some(n) = b.lines_max
                && lines.len() > n as usize
            {
                lines.truncate(n as usize);
                if let Some(l) = lines.last_mut() {
                    l.push('…');
                }
            }
            let h = lines.len() as f32 * b.st.size * b.st.line_height;
            (lines, h)
        })
        .collect();
    let content: f32 = blocks
        .iter()
        .zip(&laid)
        .map(|(b, (_, h))| b.margin.0 + h + b.margin.1)
        .sum();
    let avail = height - pad[0] - pad[2];
    let gap_n = blocks.len().saturating_sub(1).max(1) as f32;
    let (mut y, gap) = match style(root, "justifyContent") {
        Some("center") => (pad[0] + (avail - content).max(0.0) / 2.0, 0.0),
        Some("flex-end" | "end") => (pad[0] + (avail - content).max(0.0), 0.0),
        Some("space-between") => (pad[0], (avail - content).max(0.0) / gap_n),
        _ => (pad[0], 0.0),
    };
    let bg = style(root, "backgroundColor")
        .or_else(|| style(root, "background"))
        .and_then(color)
        .unwrap_or_else(|| "#fff".into());
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">\n<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>\n",
        esc(&bg)
    );
    let mut fits = vec![];
    for (b, (lines, _)) in blocks.iter().zip(&laid) {
        y += b.margin.0;
        let (x, anchor) = match (b.st.align.as_str(), rtl) {
            ("center", _) => (width / 2.0, "middle"),
            (_, true) => (width - pad[1], "end"),
            _ => (pad[3], "start"),
        };
        for line in lines {
            let base = y + b.st.size * (b.st.line_height - 1.0) / 2.0 + b.st.size * 0.8;
            svg += &format!(
                "<text x=\"{x}\" y=\"{base:.1}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" fill=\"{}\" text-anchor=\"{anchor}\"{}>{}</text>\n",
                esc(&b.st.family),
                b.st.size,
                b.st.weight,
                esc(&b.st.color),
                if rtl { " direction=\"rtl\"" } else { "" },
                esc(line)
            );
            y += b.st.size * b.st.line_height;
        }
        y += b.margin.1 + gap;
        if let Some(k) = &b.key {
            fits.push((
                k.clone(),
                Fit {
                    max_width_px: b.width,
                    max_lines: b.lines_max,
                    font: font(b),
                    text: None,
                    text_px: None,
                    occurrences: vec![],
                },
            ));
        }
    }
    svg += "</svg>\n";
    Og {
        svg,
        width,
        height,
        fits,
        overflow_px: (content - avail).max(0.0),
    }
}

fn collect(n: &Node, parent: &Inherit, box_w: f32, out: &mut Vec<Block>) {
    let Node::El { children, .. } = n else { return };
    let mut st = parent.clone();
    if let Some(f) = style(n, "fontFamily") {
        st.family = family(Some(f));
    }
    if let Some(v) = num(n, "fontSize") {
        st.size = v;
    }
    if let Some(v) = style(n, "fontWeight") {
        st.weight = weight(Some(v.to_string()));
    }
    if let Some(v) = style(n, "color") {
        st.color = v.to_string();
    }
    if let Some(v) = style(n, "lineHeight").and_then(px) {
        st.line_height = v;
    }
    if let Some(v) = style(n, "textAlign") {
        st.align = if v == "center" {
            "center".into()
        } else {
            "start".into()
        };
    }
    let pad = padding(n);
    let w = num(n, "maxWidth")
        .or_else(|| num(n, "width"))
        .unwrap_or(box_w)
        .min(box_w)
        - pad[1]
        - pad[3];
    let has_text = children
        .iter()
        .any(|c| matches!(c, Node::Text(_) | Node::Msg { .. }));
    if has_text {
        let msgs: Vec<&String> = children
            .iter()
            .filter_map(|c| match c {
                Node::Msg { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        out.push(Block {
            key: (msgs.len() == 1).then(|| msgs[0].clone()),
            text: n.text().split_whitespace().collect::<Vec<_>>().join(" "),
            width: w,
            lines_max: style(n, "lineClamp")
                .or_else(|| style(n, "WebkitLineClamp"))
                .and_then(|v| v.parse().ok()),
            margin: (
                num(n, "marginTop").unwrap_or(0.0),
                num(n, "marginBottom").unwrap_or(0.0),
            ),
            st,
        });
    } else {
        for c in children {
            collect(c, &st, w, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 360">
  <g font-family="Helvetica, Arial" font-size="20">
    <text id="step-one" x="60" y="110" font-weight="bold" data-max-width="220">Create a workspace</text>
    <text x="320" y="300" text-anchor="middle">Invite <tspan fill="red">your team</tspan></text>
    <text x="40" y="340">12</text>
  </g>
  <title>Onboarding steps</title>
</svg>"#;

    #[test]
    fn svg_texts_keys_and_budgets() {
        let t = extract("assets/onboarding.svg", SVG);
        let keys: Vec<&str> = t.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "svg.onboarding.step_one",
                "svg.onboarding.invite",
                "svg.onboarding.your_team",
                "svg.onboarding.onboarding_steps"
            ]
        );
        let f = t[0].fit.as_ref().unwrap();
        assert_eq!(
            (
                f.max_width_px,
                f.font.size_px,
                f.font.weight,
                f.font.family.as_str()
            ),
            (220.0, 20.0, 700, "Helvetica")
        );
        // middle anchor at x=320 of 640: the whole width.
        assert_eq!(t[1].fit.as_ref().unwrap().max_width_px, 640.0);
        assert_eq!(t[1].text_at, t[2].text_at);
        assert!(t[3].fit.is_none());

        let out = localize(
            SVG,
            &t,
            &|k| format!("<{}>", k.rsplit('.').next().unwrap()),
            true,
        );
        assert!(out.contains(r#"<text id="step-one" x="60" y="110" font-weight="bold" data-max-width="220">&lt;step_one&gt;</text>"#), "{out}");
        let id = text_id(&t[1]).unwrap();
        assert!(
            out.contains(&format!("<text id=\"{id}\" x=\"320\"")),
            "{out}"
        );
        assert!(out.contains("<tspan fill=\"red\">&lt;your_team&gt;</tspan>"));
        assert_eq!(text_id(&t[0]).as_deref(), Some("step-one"));
    }

    #[test]
    fn too_wide_translation_is_rejected_by_the_budget() {
        let t = extract("assets/onboarding.svg", SVG);
        let f = t[0].fit.as_ref().unwrap();
        let syntax = glot_core::validate::Syntax::Icu;
        assert!(glot_core::measure::check("Créer un espace", syntax, f).is_none());
        let issue = glot_core::measure::check(
            "Einen neuen Arbeitsbereich für Ihr ganzes Team erstellen",
            syntax,
            f,
        )
        .unwrap();
        assert_eq!(issue.code, glot_core::validate::IssueCode::TooWide);
    }

    #[test]
    fn og_layout_wraps_and_budgets() {
        let src = r##"export default function Og() {
  const t = useTranslations("og.launch");
  return (
    <div style={{ width: 1200, height: 630, padding: 80, display: "flex", flexDirection: "column", justifyContent: "center", backgroundColor: "#0b1020", color: "#fff", fontFamily: "Helvetica" }}>
      <div style={{ fontSize: 28 }}>{t("kicker")}</div>
      <div style={{ fontSize: 72, fontWeight: 700, lineClamp: 2, marginTop: 16 }}>{t("title")}</div>
    </div>
  );
}"##;
        let cat = |k: &str| match k {
            "og.launch.kicker" => Some("Now live".to_string()),
            "og.launch.title" => Some("Ship localized apps in minutes, not months".to_string()),
            _ => None,
        };
        let r = crate::email::render_jsx("og/launch.tsx", src, &cat).unwrap();
        let og = og_svg(&r.root, "ar");
        assert_eq!((og.width, og.height), (1200.0, 630.0));
        assert_eq!(og.fits.len(), 2);
        let (k, f) = &og.fits[1];
        assert_eq!(
            (k.as_str(), f.max_width_px, f.max_lines),
            ("og.launch.title", 1040.0, Some(2))
        );
        assert!(og.svg.contains("direction=\"rtl\""));
        assert!(og.svg.contains("text-anchor=\"end\""));
        assert_eq!(og.svg.matches("<text").count(), 3, "{}", og.svg);
        assert_eq!(og.overflow_px, 0.0);
    }
}
