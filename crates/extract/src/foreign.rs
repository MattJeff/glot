//! Translation calls outside JS/TS, by literal patterns (no parser): Rails `t('.x')` / `I18n.t`,
//! Django/Jinja `{% trans %}` / `_()` / `gettext()`, Laravel `__()` / `@lang` / `trans()`, Symfony
//! `->trans()` / Twig `'x'|trans`, Angular templates (`'KEY' | translate`, `[translate]`,
//! `transloco`) and Angular services (`translate.instant('KEY')`) in `.ts`.
// ponytail: a call whose key is not one string literal (variable, concatenation, interpolation) is
// skipped, never mis-keyed. Add a real parser per language if usages there must be exhaustive.
use crate::{ExtractReport, Usage};
use glot_core::context::SourceRef;

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Ruby,
    Python,
    Php,
    Twig,
    /// Angular templates, Django / Jinja templates.
    Html,
}

fn lang(path: &str) -> Option<Lang> {
    let p = path.to_ascii_lowercase();
    let ends = |exts: &[&str]| exts.iter().any(|e| p.ends_with(e));
    Some(if ends(&[".rb", ".erb", ".haml", ".slim"]) {
        Lang::Ruby
    } else if ends(&[".py"]) {
        Lang::Python
    } else if ends(&[".php"]) {
        Lang::Php
    } else if ends(&[".twig"]) {
        Lang::Twig
    } else if ends(&[".html", ".htm", ".jinja", ".jinja2", ".j2"]) {
        Lang::Html
    } else {
        return None;
    })
}

/// Files handled here instead of the JS scanner.
pub(crate) fn handles(path: &str) -> bool {
    lang(path).is_some()
}

const PY_FNS: &[&str] = &[
    "_",
    "_l",
    "gettext",
    "gettext_lazy",
    "ugettext",
    "ugettext_lazy",
    "lazy_gettext",
    "ngettext",
    "ngettext_lazy",
];

pub(crate) fn scan(path: &str, src: &str) -> ExtractReport {
    let mut s = Scan::new(path, src);
    match lang(path) {
        Some(Lang::Ruby) => s.calls(&["t", "I18n.t", "I18n.translate", "translate"], true),
        Some(Lang::Python) => s.calls(PY_FNS, false),
        Some(Lang::Php) => s.calls(
            &[
                "__",
                "trans",
                "trans_choice",
                "Lang::get",
                "Lang::choice",
                "@lang",
                "@choice",
                "->trans",
            ],
            false,
        ),
        Some(Lang::Twig) => s.pipes(&["trans"]),
        Some(Lang::Html) => {
            s.django_tags();
            s.calls(PY_FNS, false);
            s.pipes(&["translate", "transloco"]);
            s.directives();
            s.transloco_structural();
        }
        None => return ExtractReport::default(),
    }
    s.report.files_scanned = 1;
    s.report
}

/// Angular services in `.ts`: `translate.instant('KEY')`, `translateService.get('KEY')`,
/// `translocoService.translate('KEY')`, `translate('KEY')` in files importing Transloco.
pub(crate) fn angular_services(path: &str, src: &str) -> ExtractReport {
    let mut s = Scan::new(path, src);
    for method in ["instant", "get", "stream", "translate", "selectTranslate"] {
        let pat = format!(".{method}");
        for (i, _) in src.match_indices(&pat) {
            let obj: String = src[..i]
                .chars()
                .rev()
                .take_while(|c| is_ident(*c))
                .collect::<String>()
                .to_ascii_lowercase();
            let obj: String = obj.chars().rev().collect();
            if [
                "translate",
                "translateservice",
                "transloco",
                "translocoservice",
            ]
            .iter()
            .any(|o| obj.ends_with(o))
                && let Some(key) = call_arg(src, i + pat.len(), false)
            {
                s.add(key, i);
            }
        }
    }
    if src.contains("transloco") {
        s.calls(&["translate"], false);
    }
    s.report
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

struct Scan<'a> {
    path: &'a str,
    src: &'a str,
    report: ExtractReport,
}

impl<'a> Scan<'a> {
    fn new(path: &'a str, src: &'a str) -> Self {
        Self {
            path,
            src,
            report: ExtractReport::default(),
        }
    }

    fn add(&mut self, key: String, offset: usize) {
        let at = SourceRef {
            path: self.path.to_string(),
            line: self.src[..offset].matches('\n').count() as u32 + 1,
        };
        // Rails lazy lookup: `t('.title')` in `app/views/users/index.html.erb` => `users.index.title`.
        let key = match key.strip_prefix('.') {
            Some(rest) => match rails_scope(self.path) {
                Some((scope, true)) => format!("{scope}.{rest}"),
                Some((scope, false)) => {
                    // Controller: the action is not known statically.
                    self.report.dynamic_prefixes.insert(format!("{scope}."));
                    return;
                }
                None => return,
            },
            None => key,
        };
        if key.is_empty() {
            return;
        }
        self.report.usages.entry(key).or_default().push(Usage {
            at,
            ui_role: None,
            component: None,
        });
    }

    /// `name(<string>` for each name; `space_form` also accepts Ruby's `t 'key'` / `t :key`.
    fn calls(&mut self, names: &[&str], space_form: bool) {
        let src = self.src;
        for name in names {
            for (i, _) in src.match_indices(name) {
                let before = src[..i].chars().next_back();
                // `x.t(`, `my_t(`, `obj->t(` are other functions (unless the name carries its own `->`).
                let own_prefix = name.starts_with(['-', '@']);
                if !own_prefix
                    && before.is_some_and(|c| is_ident(c) || matches!(c, '.' | '>' | ':' | '@'))
                {
                    continue;
                }
                let end = i + name.len();
                if src[end..].chars().next().is_some_and(is_ident) {
                    continue;
                }
                if let Some(key) = call_arg(src, end, space_form) {
                    self.add(key, i);
                }
            }
        }
    }

    /// `{% trans "Hello" %}` / `{% translate 'Hello' %}` (Django), never `{% blocktrans %}`.
    fn django_tags(&mut self) {
        let src = self.src;
        for (i, _) in src.match_indices("{%") {
            let rest = src[i + 2..].trim_start();
            let Some(rest) = rest
                .strip_prefix("translate")
                .or_else(|| rest.strip_prefix("trans"))
            else {
                continue;
            };
            if !rest.starts_with(char::is_whitespace) {
                continue;
            }
            if let Some((key, _)) = string_at(rest.trim_start(), false) {
                self.add(key, i);
            }
        }
    }

    /// `'KEY' | translate` (Angular), `'key'|trans` (Twig): the literal right before the pipe.
    fn pipes(&mut self, names: &[&str]) {
        let src = self.src;
        let b = src.as_bytes();
        for (i, _) in src.match_indices('|') {
            if b.get(i + 1) == Some(&b'|') || (i > 0 && b[i - 1] == b'|') {
                continue;
            }
            let after = src[i + 1..].trim_start();
            if !names
                .iter()
                .any(|n| after.starts_with(n) && !after[n.len()..].starts_with(is_ident))
            {
                continue;
            }
            let before = src[..i].trim_end();
            let Some(q) = before
                .chars()
                .next_back()
                .filter(|c| matches!(c, '\'' | '"'))
            else {
                continue;
            };
            let body = &before[..before.len() - 1];
            let Some(open) = body.rfind(q) else { continue };
            let key = &body[open + 1..];
            if !key.contains('\n') && !body[..open].ends_with('\\') {
                self.add(unescape(key), open);
            }
        }
    }

    /// `[translate]="'KEY'"`, `translate="KEY"`, `<p translate>KEY</p>`, `transloco="KEY"`.
    fn directives(&mut self) {
        let src = self.src;
        for attr in ["translate", "transloco"] {
            for (i, _) in src.match_indices(attr) {
                let Some(prev) = src[..i].chars().next_back() else {
                    continue;
                };
                let after = &src[i + attr.len()..];
                if prev == '[' && after.starts_with("]=") {
                    // Bound: the value is an expression, keep it when it is one string literal.
                    let Some((outer, _)) = string_at(&after[2..], false) else {
                        continue;
                    };
                    if let Some((key, rest)) = string_at(outer.trim(), false)
                        && rest.trim().is_empty()
                    {
                        self.add(key, i);
                    }
                    continue;
                }
                if !prev.is_whitespace() || after.starts_with(is_ident) || after.starts_with('.') {
                    continue;
                }
                // Inside a tag only: the last `<` before must not be closed.
                if src[..i].rfind('<') <= src[..i].rfind('>') {
                    continue;
                }
                let after_ws = after.trim_start();
                if let Some(v) = after_ws.strip_prefix('=') {
                    if let Some((key, _)) = string_at(v.trim_start(), false)
                        && !matches!(key.as_str(), "yes" | "no" | "")
                        && !key.contains([' ', '{'])
                    {
                        self.add(key, i);
                    }
                } else if attr == "translate"
                    && let Some(gt) = after.find('>')
                    && !after[..gt].contains('<')
                {
                    // `<p translate>KEY</p>`: the content is the key.
                    let content = &after[gt + 1..];
                    let text = content[..content.find('<').unwrap_or(content.len())].trim();
                    if !text.is_empty()
                        && !text.contains(char::is_whitespace)
                        && !text.contains("{{")
                    {
                        self.add(text.to_string(), i);
                    }
                }
            }
        }
    }

    /// `*transloco="let t; read: 'scope'"` then `t('key')` in the template => `scope.key`.
    fn transloco_structural(&mut self) {
        let src = self.src;
        let Some(i) = src.find("*transloco=") else {
            return;
        };
        let Some((spec, _)) = string_at(&src[i + "*transloco=".len()..], false) else {
            return;
        };
        let Some(var) = spec
            .split(';')
            .find_map(|p| p.trim().strip_prefix("let "))
            .map(str::trim)
        else {
            return;
        };
        let read = spec
            .split(';')
            .find_map(|p| p.trim().strip_prefix("read:"))
            .and_then(|r| string_at(r.trim(), false))
            .map(|(r, _)| r);
        let mut inner = Scan::new(self.path, src);
        inner.calls(&[var], false);
        for (k, v) in inner.report.usages {
            let k = match &read {
                Some(r) => format!("{r}.{k}"),
                None => k,
            };
            self.report.usages.entry(k).or_default().extend(v);
        }
    }
}

/// The string literal (or Ruby symbol) argument right after a function name at `end`.
fn call_arg(src: &str, end: usize, space_form: bool) -> Option<String> {
    let rest = &src[end..];
    let arg = match rest.trim_start().strip_prefix('(') {
        Some(a) => a.trim_start(),
        None if space_form && rest.starts_with([' ', '\t']) => rest.trim_start(),
        None => return None,
    };
    let (key, after) = string_at(arg, space_form)?;
    // `t("a" + x)` / `_("a" "b")`: not one literal.
    let tail = after.trim_start_matches([' ', '\t']);
    (tail.is_empty() || tail.starts_with([')', ',', '%', '}', '|', '-', '\n', '\r'])).then_some(key)
}

/// A quoted literal at the start of `s` (or a Ruby `:symbol` when `symbols`), and what follows.
/// Interpolated strings (`"#{x}"`, PHP `"$x"`) are `None`.
fn string_at(s: &str, symbols: bool) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    let (_, q) = chars.next()?;
    if symbols && q == ':' {
        let n = s[1..]
            .find(|c: char| !(is_ident(c) || c == '?' || c == '!'))
            .map_or(s.len(), |n| n + 1);
        return (n > 1).then(|| (s[1..n].to_string(), &s[n..]));
    }
    if !matches!(q, '\'' | '"') {
        return None;
    }
    let mut escaped = false;
    for (i, c) in chars {
        match c {
            '\n' => return None,
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            c if c == q => {
                let body = &s[1..i];
                if q == '"' && (body.contains("#{") || body.contains('$')) {
                    return None;
                }
                return Some((unescape(body), &s[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some(n) => out.push(n),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Rails lazy-lookup scope: views => (`users.index`, exact), controllers => (`admin.users`, prefix only).
fn rails_scope(path: &str) -> Option<(String, bool)> {
    let p = path.replace('\\', "/");
    if let Some(v) = p.split_once("app/views/").map(|(_, v)| v) {
        let (dir, file) = v.rsplit_once('/').unwrap_or(("", v));
        let file = file.split('.').next()?.trim_start_matches('_');
        let scope = if dir.is_empty() {
            file.to_string()
        } else {
            format!("{}.{file}", dir.replace('/', "."))
        };
        return Some((scope, true));
    }
    let c = p
        .split_once("app/controllers/")?
        .1
        .strip_suffix("_controller.rb")?;
    Some((c.replace('/', "."), false))
}

/// Vue SFC `<template>`: `$t('key')` / `t('key')` / `$tc` / `$te`, `v-t="'key'"`, and
/// `<i18n-t keypath="key">`. `<script>` blocks are left to the JS scanner.
pub(crate) fn vue_template(path: &str, src: &str) -> ExtractReport {
    let (Some(a), Some(b)) = (src.find("<template"), src.rfind("</template>")) else {
        return ExtractReport::default();
    };
    // Blank everything outside the template so offsets and lines stay those of the file.
    let tpl: String = src
        .char_indices()
        .map(|(i, c)| {
            if (a..b).contains(&i) || c == '\n' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut s = Scan::new(path, &tpl);
    s.calls(&["$t", "t", "$tc", "tc", "$te", "te"], false);
    for attr in ["v-t=", "keypath=", "path="] {
        for (i, _) in tpl.match_indices(attr) {
            if tpl[..i].ends_with(|c: char| is_ident(c) || c == '-' || c == ':') {
                continue;
            }
            let Some((v, _)) = string_at(&tpl[i + attr.len()..], false) else {
                continue;
            };
            // `v-t` is an expression: one string literal only.
            let key = if attr == "v-t=" {
                match string_at(v.trim(), false) {
                    Some((k, rest)) if rest.trim().is_empty() => k,
                    _ => continue,
                }
            } else {
                v
            };
            s.add(key, i);
        }
    }
    s.report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(path: &str, src: &str) -> Vec<String> {
        let r = if handles(path) {
            scan(path, src)
        } else {
            angular_services(path, src)
        };
        r.usages.keys().cloned().collect()
    }

    #[test]
    fn rails() {
        let erb = "<h1><%= t('.title') %></h1>\n<p><%= t \"users.greet\", name: @u %></p>\n<%= t(:plain) %> <%= I18n.t('a.b') %> <%= t(\"x#{y}\") %> <%= format_t('no') %>";
        assert_eq!(
            keys("app/views/users/index.html.erb", erb),
            ["a.b", "plain", "users.greet", "users.index.title"]
        );
        assert_eq!(
            keys("app/views/users/_form.html.erb", "<%= t '.save' %>"),
            ["users.form.save"]
        );
        let r = scan(
            "app/controllers/admin/users_controller.rb",
            "flash[:notice] = t('.created')\nI18n.t(\"errors.x\")",
        );
        assert_eq!(r.usages.keys().collect::<Vec<_>>(), ["errors.x"]);
        assert!(r.dynamic_prefixes.contains("admin.users."));
        assert_eq!(r.usages["errors.x"][0].at.line, 2);
    }

    #[test]
    fn django_and_jinja() {
        let py = "from django.utils.translation import gettext as _\nmsg = _(\"Welcome back\")\nx = gettext_lazy('Save')\ny = ngettext('One item', '%(n)d items', n)\nz = my_(\"no\")\nf = _(f\"no {x}\")";
        assert_eq!(
            keys("app/views.py", py),
            ["One item", "Save", "Welcome back"]
        );
        let html = "{% load i18n %}<h1>{% trans \"Hello\" %}</h1>{% translate 'Bye' as b %}{% blocktrans %}x{% endblocktrans %}{{ _('Jinja') }}";
        assert_eq!(keys("templates/home.html", html), ["Bye", "Hello", "Jinja"]);
    }

    #[test]
    fn laravel_and_symfony() {
        let php = "<?php echo __('messages.welcome'); ?> @lang('auth.failed') {{ trans_choice('apples', 3) }} {{ __(\"Hi $name\") }} $t->trans('sym.key'); my__('no');";
        assert_eq!(
            keys("resources/views/home.blade.php", php),
            ["apples", "auth.failed", "messages.welcome", "sym.key"]
        );
        assert_eq!(
            keys(
                "templates/a.html.twig",
                "{{ 'nav.home'|trans }} {{ a || b }}"
            ),
            ["nav.home"]
        );
    }

    #[test]
    fn angular() {
        let html = r#"<h1>{{ 'HOME.TITLE' | translate }}</h1>
<p [translate]="'HOME.INTRO'" [translateParams]="{v: 1}"></p>
<p translate>HOME.BODY</p>
<p translate="no">Brand</p>
<span transloco="nav.home"></span>
<b>{{ x || 'y' }}</b>
<ng-container *transloco="let t; read: 'dash'"><i>{{ t('title') }}</i></ng-container>"#;
        assert_eq!(
            keys("src/app/home.component.html", html),
            [
                "HOME.BODY",
                "HOME.INTRO",
                "HOME.TITLE",
                "dash.title",
                "nav.home"
            ]
        );
        let ts = "this.translate.instant('A.B'); this.translateService.get(\"C\"); this.translocoService.translate('d.e'); cache.get('nope');";
        assert_eq!(keys("src/app/x.ts", ts), ["A.B", "C", "d.e"]);
    }
}
