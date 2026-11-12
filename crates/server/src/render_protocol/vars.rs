//! Variables: applied by glot after translation, in memory, never stored, never sent to a model.
//!
//! - `{{name}}` (Mustache / Handlebars / Liquid output) in text, HTML, Markdown and JSON fields: HTML-escaped in
//!   HTML, `{{{name}}}` inserts as is. A variable that is not given is left in place (your email provider may
//!   fill it later).
//! - ICU MessageFormat in `template`: `{name}`, `{n, plural, …}`, `{g, select, …}`, `{n, number}`, `{d, date, long}`.
//! - Typed values are formatted for the recipient's locale: `{"type": "currency", "value": 1234.5, "currency": "EUR"}`,
//!   `number`, `percent`, `date`, `datetime`.
use chrono::{Datelike, Timelike};
use icu_datetime::{
    DateTimeFormatter, fieldsets,
    input::{Date, DateTime, Time},
};
use icu_decimal::{DecimalFormatter, input::Decimal};
use icu_locale_core::Locale;
use serde_json::{Map, Value};

pub type Vars = Map<String, Value>;

fn locale(l: &str) -> Locale {
    let tag = l.replace('_', "-");
    tag.parse()
        .or_else(|_| glot_core::plural::language(&tag).parse())
        .unwrap_or(Locale::UNKNOWN)
}

/// `user.name` → `vars["user"]["name"]`; a typed value (`{"type": …, "value": …}`) is a leaf.
fn lookup<'a>(vars: &'a Vars, path: &str) -> Option<&'a Value> {
    if let Some(v) = vars.get(path) {
        return Some(v);
    }
    let mut parts = path.split('.');
    let mut v = vars.get(parts.next()?)?;
    for p in parts {
        v = v.as_object()?.get(p)?;
    }
    Some(v)
}

/// Check every variable once, before any work: a bad date or an unknown type is a 400, not a broken email.
pub fn validate(vars: &Vars, loc: &str) -> Result<(), String> {
    for (k, v) in vars {
        if v.get("type").is_some() {
            format(v, loc).map_err(|e| format!("variable '{k}': {e}"))?;
        }
    }
    Ok(())
}

/// A variable as text for `loc`.
pub fn format(v: &Value, loc: &str) -> Result<String, String> {
    let o = match v {
        Value::String(s) => return Ok(s.clone()),
        Value::Null => return Ok(String::new()),
        Value::Bool(b) => return Ok(b.to_string()),
        // A bare number is an identifier as often as a quantity: inserted as written. Type it to format it.
        Value::Number(n) => return Ok(n.to_string()),
        Value::Array(_) => return Err("arrays are not supported".into()),
        Value::Object(o) => o,
    };
    let ty = o
        .get("type")
        .and_then(Value::as_str)
        .ok_or("an object variable needs a `type` (number, currency, percent, date, datetime)")?;
    let value = o.get("value").ok_or("missing `value`")?;
    let digits = |k: &str| o.get(k).and_then(Value::as_u64).map(|d| d.min(20) as i16);
    let num = || -> Result<f64, String> {
        match value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
        .filter(|f| f.is_finite())
        .ok_or_else(|| format!("`value` must be a number for type '{ty}'"))
    };
    match ty {
        "number" => Ok(number(
            num()?,
            loc,
            digits("min_fraction").unwrap_or(0),
            digits("max_fraction").unwrap_or(3),
        )),
        "percent" => Ok(percent(num()?, loc, digits("max_fraction").unwrap_or(0))),
        "currency" => {
            let code = o
                .get("currency")
                .and_then(Value::as_str)
                .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic()))
                .ok_or("type 'currency' needs `currency`: an ISO 4217 code such as \"EUR\"")?;
            Ok(currency(num()?, &code.to_ascii_uppercase(), loc))
        }
        "date" | "datetime" => {
            let style = o.get("style").and_then(Value::as_str).unwrap_or("medium");
            date(value, ty == "datetime", style, loc)
        }
        t => Err(format!(
            "unknown type '{t}' (number, currency, percent, date, datetime)"
        )),
    }
}

// ponytail: a formatter is built per value (a CLDR data lookup); cache per locale if a profile ever shows it.
fn number(n: f64, loc: &str, min_fraction: i16, max_fraction: i16) -> String {
    let max = max_fraction.max(min_fraction);
    // Through the shortest decimal text of the float: 0.1 + 0.2 style noise is cut by the rounding below.
    let Ok(mut d) = Decimal::try_from_str(&format!("{:.*}", (max as usize).min(20), n)) else {
        return n.to_string();
    };
    d.round(-max);
    d.trim_end();
    d.pad_end(-min_fraction);
    match DecimalFormatter::try_new(locale(loc).into(), Default::default()) {
        Ok(f) => f.format(&d).to_string(),
        Err(_) => d.to_string(),
    }
}

const NBSP: char = '\u{a0}';

fn percent(ratio: f64, loc: &str, max_fraction: i16) -> String {
    let n = number(ratio * 100.0, loc, 0, max_fraction);
    // ponytail: CLDR percent patterns of the common languages by hand (ICU4X percent formatting is still
    // experimental); switch to it when it is stable.
    match glot_core::plural::language(loc).as_str() {
        "fr" => format!("{n}\u{202f}%"),
        "de" | "es" | "ru" | "sv" | "nb" | "no" | "da" | "fi" | "cs" | "sk" | "ro" | "sl"
        | "bg" | "be" | "eu" | "is" | "lt" | "et" => format!("{n}{NBSP}%"),
        "tr" => format!("%{n}"),
        _ => format!("{n}%"),
    }
}

fn currency(amount: f64, code: &str, loc: &str) -> String {
    let fraction = match code {
        "JPY" | "KRW" | "VND" | "CLP" | "ISK" | "UGX" | "XAF" | "XOF" | "PYG" | "RWF" | "KMF"
        | "GNF" | "DJF" | "VUV" | "XPF" => 0,
        "BHD" | "KWD" | "OMR" | "JOD" | "TND" => 3,
        _ => 2,
    };
    let n = number(amount.abs(), loc, fraction, fraction);
    let lang = glot_core::plural::language(loc);
    let region = loc
        .split(['-', '_'])
        .skip(1)
        .find(|s| s.len() == 2)
        .unwrap_or("");
    let symbol = match (code, region) {
        ("EUR", _) => "€",
        ("GBP", _) => "£",
        ("INR", _) => "₹",
        ("KRW", _) => "₩",
        ("ILS", _) => "₪",
        ("UAH", _) => "₴",
        ("PLN", _) if lang == "pl" => "zł",
        ("BRL", _) => "R$",
        ("JPY", _) if lang == "ja" => "￥",
        ("JPY", _) => "¥",
        ("CNY", _) if lang == "zh" => "¥",
        // "$" alone only where it is the local dollar; elsewhere the code avoids charging the wrong dollar.
        ("USD", "US" | "") if lang == "en" => "$",
        ("USD", _) if !matches!(region, "CA" | "AU" | "NZ" | "SG" | "HK" | "MX") => "$",
        ("CAD", "CA") | ("AUD", "AU") | ("NZD", "NZ") | ("MXN", "MX") => "$",
        _ => code,
    };
    let sign = if amount < 0.0 { "-" } else { "" };
    // ponytail: CLDR currency patterns of the common languages by hand: symbol before (`$1,234.50`), before with
    // a space (`€ 1.234,50`), or after with a no-break space (`1 234,50 €`). ICU4X's CurrencyFormatter is
    // experimental; switch to it when it is stable. Unknown languages get the English pattern.
    match lang.as_str() {
        "nl" | "nb" | "no" => format!("{symbol}{NBSP}{sign}{n}"),
        "pt" if region == "BR" || region.is_empty() => format!("{sign}{symbol}{NBSP}{n}"),
        "fr" | "de" | "es" | "it" | "pt" | "ru" | "pl" | "sv" | "da" | "fi" | "cs" | "sk"
        | "uk" | "el" | "ro" | "hu" | "bg" | "hr" | "sl" | "lt" | "lv" | "et" | "ca" | "vi"
        | "tr" | "sr" | "is" | "eu" | "gl" | "be" | "ar" | "he" | "fa" => {
            format!("{sign}{n}{NBSP}{symbol}")
        }
        _ if symbol.len() == 3 && symbol == code => format!("{sign}{symbol}{NBSP}{n}"),
        _ => format!("{sign}{symbol}{n}"),
    }
}

fn date(value: &Value, with_time: bool, style: &str, loc: &str) -> Result<String, String> {
    use chrono::{DateTime as Cdt, NaiveDate, NaiveDateTime};
    const HELP: &str = "expected \"2026-11-11\", an RFC 3339 datetime (\"2026-11-11T09:30:00+01:00\", shown at that \
                        offset) or Unix seconds";
    let naive: NaiveDateTime = match value {
        Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            // ponytail: the wall clock of the offset written in the value; no IANA time zone database here.
            // Convert to the recipient's offset before calling, or add chrono-tz when a `time_zone` option is needed.
            .or_else(|| Cdt::parse_from_rfc3339(s).ok().map(|d| d.naive_local()))
            .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
            .ok_or_else(|| format!("'{s}' is not a date: {HELP}"))?,
        Value::Number(n) => n
            .as_i64()
            .and_then(|s| Cdt::from_timestamp(s, 0))
            .map(|d| d.naive_utc())
            .ok_or(HELP)?,
        _ => return Err(HELP.into()),
    };
    let bad = |e: &dyn std::fmt::Display| format!("cannot format the date: {e}");
    let d = Date::try_new_iso(naive.year(), naive.month() as u8, naive.day() as u8)
        .map_err(|e| bad(&e))?;
    let ymd = match style {
        "short" => fieldsets::YMD::short(),
        "medium" => fieldsets::YMD::medium(),
        "long" | "full" => fieldsets::YMD::long(),
        s => return Err(format!("unknown style '{s}' (short, medium, long)")),
    };
    let l = locale(loc);
    if with_time {
        let time =
            Time::try_new(naive.hour() as u8, naive.minute() as u8, 0, 0).map_err(|e| bad(&e))?;
        let f = DateTimeFormatter::try_new(l.into(), ymd.with_time_hm()).map_err(|e| bad(&e))?;
        Ok(f.format(&DateTime { date: d, time }).to_string())
    } else {
        let f = DateTimeFormatter::try_new(l.into(), ymd).map_err(|e| bad(&e))?;
        Ok(f.format(&d).to_string())
    }
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Fill `{{name}}` (through `escape`) and `{{{name}}}` (as is). Unknown names, block tags and helpers stay.
pub fn mustache(s: &str, vars: &Vars, loc: &str, escape: fn(&str) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        let raw = rest[i..].starts_with("{{{");
        let (open, close) = if raw { (3, "}}}") } else { (2, "}}") };
        let Some(j) = rest[i + open..].find(close) else {
            break;
        };
        let end = i + open + j + close.len();
        let name = rest[i + open..i + open + j].trim();
        let simple = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'));
        out.push_str(&rest[..i]);
        match lookup(vars, name)
            .filter(|_| simple)
            .and_then(|v| format(v, loc).ok())
        {
            Some(v) if raw => out.push_str(&v),
            Some(v) => out.push_str(&escape(&v)),
            None => out.push_str(&rest[i..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Index of the `}` closing the `{` at byte `open` (ICU apostrophe quoting respected).
fn matching(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i, mut quoted) = (0, open, false);
    while i < b.len() {
        match b[i] {
            b'\'' if b.get(i + 1) == Some(&b'\'') => i += 1,
            b'\'' if quoted => quoted = false,
            b'\'' if matches!(b.get(i + 1), Some(b'{' | b'}')) => quoted = true,
            b'{' if !quoted => depth += 1,
            b'}' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `sel {message}` pairs of a plural / select body.
fn branches(body: &str) -> Result<Vec<(&str, &str)>, String> {
    let mut out = vec![];
    let mut at = 0;
    while let Some(rel) = body[at..].find('{') {
        let open = at + rel;
        let close = matching(body, open).ok_or("unbalanced braces")?;
        out.push((body[at..open].trim(), &body[open + 1..close]));
        at = close + 1;
    }
    Ok(out)
}

/// Format an ICU MessageFormat `msg` with `vars` for `loc`. Errors name the argument.
pub fn icu(msg: &str, vars: &Vars, loc: &str) -> Result<String, String> {
    let mut out = String::new();
    icu_into(msg, vars, loc, None, &mut out)?;
    Ok(out)
}

fn icu_into(
    msg: &str,
    vars: &Vars,
    loc: &str,
    pound: Option<&str>,
    out: &mut String,
) -> Result<(), String> {
    let b = msg.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\'' if b.get(i + 1) == Some(&b'\'') => {
                out.push('\'');
                i += 2;
            }
            b'\'' if matches!(b.get(i + 1), Some(b'{' | b'}' | b'#' | b'|')) => {
                // Quoted literal up to the next lone apostrophe.
                let mut j = i + 1;
                while j < b.len() {
                    if b[j] == b'\'' {
                        if b.get(j + 1) == Some(&b'\'') {
                            out.push('\'');
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    let c = msg[j..].chars().next().expect("in bounds");
                    out.push(c);
                    j += c.len_utf8();
                }
                i = j + 1;
            }
            b'#' if pound.is_some() => {
                out.push_str(pound.unwrap_or_default());
                i += 1;
            }
            b'{' => {
                let close = matching(msg, i).ok_or("unbalanced braces in the template")?;
                argument(&msg[i + 1..close], vars, loc, out)?;
                i = close + 1;
            }
            _ => {
                let c = msg[i..].chars().next().expect("in bounds");
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    Ok(())
}

fn argument(inner: &str, vars: &Vars, loc: &str, out: &mut String) -> Result<(), String> {
    let mut parts = inner.splitn(3, ',');
    let name = parts.next().unwrap_or("").trim();
    let kind = parts.next().map(str::trim);
    let rest = parts.next().unwrap_or("").trim();
    let v = lookup(vars, name).ok_or_else(|| format!("missing variable '{name}'"))?;
    let as_number = || -> Result<f64, String> {
        match v.get("value").unwrap_or(v) {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
        .ok_or_else(|| format!("variable '{name}' must be a number"))
    };
    match kind {
        None => out.push_str(&format(v, loc).map_err(|e| format!("variable '{name}': {e}"))?),
        Some("plural" | "selectordinal") => {
            let n = as_number()?;
            let (offset, body) = match rest.strip_prefix("offset:") {
                Some(r) => {
                    let (o, b) = r
                        .trim_start()
                        .split_once(char::is_whitespace)
                        .unwrap_or((r, ""));
                    (o.parse::<f64>().unwrap_or(0.0), b)
                }
                None => (0.0, rest),
            };
            let opts = branches(body).map_err(|e| format!("argument '{name}': {e}"))?;
            let shown = n - offset;
            let category =
                glot_core::plural::category(loc, &shown.to_string(), kind == Some("selectordinal"));
            let pick = opts
                .iter()
                .find(|(s, _)| s.strip_prefix('=').and_then(|x| x.parse::<f64>().ok()) == Some(n))
                .or_else(|| {
                    opts.iter()
                        .find(|(s, _)| category.is_some_and(|c| c.as_str() == *s))
                })
                .or_else(|| opts.iter().find(|(s, _)| *s == "other"))
                .ok_or_else(|| format!("argument '{name}': no `other` branch"))?;
            icu_into(pick.1, vars, loc, Some(&number(shown, loc, 0, 3)), out)?;
        }
        Some("select") => {
            let s = format(v, loc).map_err(|e| format!("variable '{name}': {e}"))?;
            let opts = branches(rest).map_err(|e| format!("argument '{name}': {e}"))?;
            let pick = opts
                .iter()
                .find(|(k, _)| *k == s)
                .or_else(|| opts.iter().find(|(k, _)| *k == "other"))
                .ok_or_else(|| format!("argument '{name}': no `other` branch"))?;
            icu_into(pick.1, vars, loc, None, out)?;
        }
        Some("number") if v.get("type").is_none() => out.push_str(&match rest {
            "percent" => percent(as_number()?, loc, 0),
            "integer" => number(as_number()?, loc, 0, 0),
            _ => number(as_number()?, loc, 0, 3),
        }),
        Some("date" | "time") if v.get("type").is_none() => {
            let style = if rest.is_empty() { "medium" } else { rest };
            out.push_str(
                &date(v, kind == Some("time"), style, loc)
                    .map_err(|e| format!("variable '{name}': {e}"))?,
            );
        }
        // A typed value carries its own format.
        Some("number" | "date" | "time") => {
            out.push_str(&format(v, loc).map_err(|e| format!("variable '{name}': {e}"))?)
        }
        Some(k) => return Err(format!("argument '{name}': unsupported format '{k}'")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars(v: Value) -> Vars {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn numbers_dates_currencies_follow_the_locale() {
        let f = |v: Value, l: &str| format(&v, l).unwrap();
        let money = json!({"type": "currency", "value": 1234.5, "currency": "EUR"});
        assert_eq!(f(money.clone(), "en-US"), "€1,234.50");
        assert_eq!(f(money.clone(), "fr-FR"), "1\u{202f}234,50\u{a0}€");
        assert_eq!(f(money, "de-DE"), "1.234,50\u{a0}€");
        assert_eq!(
            f(
                json!({"type": "currency", "value": 1980, "currency": "JPY"}),
                "ja"
            ),
            "￥1,980"
        );
        assert_eq!(
            f(json!({"type": "number", "value": 1234567.891}), "de"),
            "1.234.567,891"
        );
        assert_eq!(
            f(json!({"type": "percent", "value": 0.256}), "fr"),
            "26\u{202f}%"
        );
        assert_eq!(f(json!({"type": "percent", "value": 0.256}), "en"), "26%");
        let d = json!({"type": "date", "value": "2026-11-11", "style": "long"});
        assert_eq!(f(d.clone(), "en-US"), "November 11, 2026");
        assert_eq!(f(d.clone(), "fr"), "11 novembre 2026");
        assert_eq!(f(d, "ja"), "2026年11月11日");
        let dt =
            json!({"type": "datetime", "value": "2026-11-11T09:30:00+01:00", "style": "short"});
        assert!(f(dt, "fr").contains("09:30"));
        // Bare values are inserted as written: an order number is not a quantity.
        assert_eq!(f(json!(1234567), "fr"), "1234567");
        assert!(
            format(&json!({"type": "date", "value": "tomorrow"}), "fr")
                .unwrap_err()
                .contains("not a date")
        );
        assert!(
            format(&json!({"type": "currency", "value": 1}), "fr")
                .unwrap_err()
                .contains("ISO 4217")
        );
    }

    #[test]
    fn mustache_escapes_in_html_and_leaves_unknown_tokens() {
        let v = vars(
            json!({"name": "<b>Zoë</b> & co", "user": {"city": "Lyon"}, "total": {"type": "currency", "value": 9.5, "currency": "USD"}}),
        );
        let s =
            "Hi {{name}} from {{ user.city }} — {{{name}}} — {{total}} {{#if x}}{{unknown}}{{/if}}";
        assert_eq!(
            mustache(s, &v, "en-US", escape_html),
            "Hi &lt;b&gt;Zoë&lt;/b&gt; &amp; co from Lyon — <b>Zoë</b> & co — $9.50 {{#if x}}{{unknown}}{{/if}}"
        );
    }

    #[test]
    fn icu_plural_select_and_quotes() {
        let m = "{name}, {count, plural, =0 {no files} one {# file} other {# files}} for {g, select, f {her} other {them}}. It''s '{literal}'";
        let v = |n: i64| vars(json!({"name": "Ada", "count": n, "g": "f"}));
        assert_eq!(
            icu(m, &v(0), "en").unwrap(),
            "Ada, no files for her. It's {literal}"
        );
        assert_eq!(
            icu(m, &v(1), "en").unwrap(),
            "Ada, 1 file for her. It's {literal}"
        );
        assert_eq!(
            icu(m, &v(12345), "en").unwrap(),
            "Ada, 12,345 files for her. It's {literal}"
        );
        // Polish picks `few` for 3; the message only has one/other in this test, so `other`.
        let pl = "{n, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}";
        assert_eq!(icu(pl, &vars(json!({"n": 3})), "pl").unwrap(), "3 pliki");
        assert_eq!(icu(pl, &vars(json!({"n": 5})), "pl").unwrap(), "5 plików");
        assert_eq!(
            icu("{d, date, long}", &vars(json!({"d": "2026-11-11"})), "es").unwrap(),
            "11 de noviembre de 2026"
        );
        assert!(
            icu("{who}", &Vars::new(), "en")
                .unwrap_err()
                .contains("missing variable 'who'")
        );
    }
}
