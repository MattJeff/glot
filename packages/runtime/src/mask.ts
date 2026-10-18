// Pure masking / restoration: no DOM, no Translator. Runs in Node for the unit tests.
//
// A message is split into translation *units*: its top-level text, then each plural/select branch
// separately (recursively). Inside a unit, everything that must survive verbatim (placeholders, tags,
// `#`, nested plurals, glossary terms) becomes a stable token `⟦n⟧`. After translation the tokens are
// put back; a unit whose tokens don't come back exactly once each is rejected (→ source fallback).

export type Syntax = "icu" | "i18next" | "mf2";

export interface Term {
  term: string;
  do_not_translate?: boolean;
  case_sensitive?: boolean;
  translations?: Record<string, string>;
  description?: string;
  forbidden?: Record<string, string[]>;
}
/** `glossary.toml` parsed (`{ term: [...] }`) or the API's `Term[]`. */
export type Glossary = Term[] | { term?: Term[] };

export const terms = (g?: Glossary): Term[] => (Array.isArray(g) ? g : g?.term ?? []);

type Node = string | { raw: string } | Choice;
interface Choice {
  head: string; // `{count, plural, offset:1 ` verbatim
  kind: "plural" | "selectordinal" | "select";
  options: [string, Node[]][];
  pound: boolean; // `#` is special inside the branches
}

// ---------------------------------------------------------------- parsing

/** ICU MessageFormat (FormatJS flavour: tags, DOUBLE_OPTIONAL apostrophes). Throws on syntax errors. */
export function parseIcu(s: string): Node[] {
  let i = 0;
  const fail = (m: string): never => { throw new Error(`${m} at ${i}`); };
  const ws = () => { while (/\s/.test(s[i] ?? "")) i++; };

  function seq(pound: boolean, depth: number): Node[] {
    const out: Node[] = [];
    let text = "";
    const flush = () => { if (text) out.push(text); text = ""; };
    while (i < s.length) {
      const c = s[i];
      if (c === "{") { flush(); out.push(arg(pound, depth)); }
      else if (c === "}") { if (depth === 0) fail("unmatched }"); break; }
      else if (c === "#" && pound) { flush(); out.push({ raw: "#" }); i++; }
      else if (c === "<") {
        const m = /^<\/?[\p{L}\p{N}_.-]+ *\/?>/u.exec(s.slice(i));
        if (m) { flush(); out.push({ raw: m[0] }); i += m[0].length; } else { text += c; i++; }
      } else if (c === "'") {
        i++;
        const n = s[i];
        if (n === "'") { text += "'"; i++; }
        else if (n !== undefined && ("{}<>".includes(n) || (pound && n === "#"))) {
          while (i < s.length) {
            const q = s[i++];
            if (q !== "'") text += q;
            else if (s[i] === "'") { text += "'"; i++; }
            else break;
          }
        } else text += "'";
      } else { text += c; i++; }
    }
    flush();
    return out;
  }

  function arg(pound: boolean, depth: number): Node {
    const start = i;
    const m = /^\{\s*[^\s,{}]+\s*(?:,\s*(\w+)\s*(,)?)?/.exec(s.slice(i)) ?? fail("bad argument");
    const kind = m[1];
    if (m[2] && (kind === "plural" || kind === "selectordinal" || kind === "select")) {
      i += m[0].length;
      ws();
      const off = /^offset:\s*\d+\s*/.exec(s.slice(i));
      if (off) i += off[0].length;
      const head = s.slice(start, i);
      const inner = kind === "select" ? pound : true;
      const options: [string, Node[]][] = [];
      for (;;) {
        ws();
        if (s[i] === "}") { i++; break; }
        const k = /^[^\s{}]+/.exec(s.slice(i)) ?? fail("bad option");
        i += k[0].length;
        ws();
        if (s[i++] !== "{") fail("expected {");
        options.push([k[0], seq(inner, depth + 1)]);
        if (s[i++] !== "}") fail("unclosed option");
      }
      return { head, kind, options, pound: inner };
    }
    // Simple / formatted argument: copy verbatim up to the matching brace.
    let d = 0;
    while (i < s.length) {
      const c = s[i++];
      if (c === "{") d++;
      else if (c === "}" && --d === 0) return { raw: s.slice(start, i) };
    }
    return fail("unclosed argument");
  }

  return seq(false, 0);
}

/** i18next: `{{x}}`, `$t(key)`, `<0>`/`<b>` tags. Plurals are separate `_one`/`_other` keys. */
const I18N = /\{\{[^}]*\}\}|\$t\([^)]*\)|<\/?[\p{L}\p{N}_.-]+ *\/?>/gu;
/** MF2 simple message: expressions/markup `{…}` and escapes `\{`. Complex messages (`.match`…) are skipped. */
const MF2 = /\\.|\{[^}]*\}/g;

function parseFlat(s: string, re: RegExp): Node[] {
  const out: Node[] = [];
  let last = 0;
  for (const m of s.matchAll(re)) {
    if (m.index > last) out.push(s.slice(last, m.index));
    out.push({ raw: m[0] });
    last = m.index + m[0].length;
  }
  if (last < s.length) out.push(s.slice(last));
  return out;
}

export function parse(s: string, syntax: Syntax): Node[] | null {
  try {
    if (syntax === "icu") return parseIcu(s);
    if (syntax === "i18next") return parseFlat(s, I18N);
    // ponytail: MF2 `.input/.local/.match` messages stay in the source language.
    if (/^\s*\./.test(s) || s.trimStart().startsWith("{{")) return null;
    return parseFlat(s, MF2);
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- escaping

function escape(text: string, syntax: Syntax, pound: boolean): string {
  if (syntax === "i18next") return text;
  if (syntax === "mf2") return text.replace(/[\\{}|]/g, (c) => "\\" + c);
  const special = pound ? /[{}<>#]/g : /[{}<>]/g;
  return text.replace(/'/g, "''").replace(special, (c) => `'${c}'`);
}

// ---------------------------------------------------------------- glossary

const reEscape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
export const lookup = <T>(m: Record<string, T> | undefined, locale: string): T | undefined =>
  m?.[locale] ?? m?.[locale.split(/[-_]/)[0]];

/** Word-boundary match, like glot-core's `Glossary::matches`. */
function termRe(t: Term): RegExp {
  return new RegExp(`(?<![\\p{L}\\p{N}_])${reEscape(t.term)}(?![\\p{L}\\p{N}_])`, t.case_sensitive ? "gu" : "giu");
}

// ---------------------------------------------------------------- units

const TOKEN = /⟦\s*(\d+)\s*⟧/g;
type Slot = { raw: string } | Choice;
export interface Unit {
  /** Full masked text (tokens included). */
  text: string;
  lead: string;
  /** What goes to the translator. Empty => nothing to translate (no letters). */
  core: string;
  trail: string;
  slots: Slot[];
  syntax: Syntax;
  pound: boolean;
}

export interface Masked {
  /** Every unit of the message, branches included (dedupe on `core` before translating). */
  units: Unit[];
  /** Rebuild the message from `translate(unit) → string`; null if any unit lost/duplicated a token. */
  restore(get: (u: Unit) => string): string | null;
}

/** Mask one message for `locale`. Returns null when the message can't be parsed (it stays in source). */
export function mask(message: string, opts: { syntax?: Syntax; glossary?: Glossary; locale: string }): Masked | null {
  const syntax = opts.syntax ?? "icu";
  const nodes = parse(message, syntax);
  if (!nodes) return null;
  // Longest first so "Team workspace" wins over "Team".
  const gl = terms(opts.glossary)
    .filter((t) => t.term && (t.do_not_translate || lookup(t.translations, opts.locale)))
    .sort((a, b) => b.term.length - a.term.length);
  const units: Unit[] = [];
  const unitOf = new Map<Node[], Unit>();

  function build(list: Node[], pound: boolean): Unit {
    const slots: Slot[] = [];
    const tok = (s: Slot) => `⟦${slots.push(s) - 1}⟧`;
    let text = "";
    for (const n of list) {
      if (typeof n !== "string") {
        if ("options" in n) {
          for (const [, b] of n.options) build(b, n.pound);
          text += tok(n);
        } else text += tok(n);
        continue;
      }
      let piece = n;
      for (const t of gl) {
        piece = piece.replace(termRe(t), (hit) =>
          tok({ raw: escape(t.do_not_translate ? hit : lookup(t.translations, opts.locale)!, syntax, pound) }),
        );
      }
      text += piece;
    }
    const core = text.trim();
    const u: Unit = {
      text,
      lead: text.slice(0, text.length - text.trimStart().length),
      core: /\p{L}/u.test(core.replace(TOKEN, "")) ? core : "",
      trail: text.slice(text.trimEnd().length),
      slots, syntax, pound,
    };
    units.push(u);
    unitOf.set(list, u);
    return u;
  }
  const top = build(nodes, false);

  function render(u: Unit, get: (u: Unit) => string): string {
    if (!u.core) return replay(u, u.text, get); // nothing to translate: original text + slots
    const out = get(u);
    const seen = [...out.matchAll(TOKEN)].map((m) => +m[1]).sort((a, b) => a - b);
    if (seen.join() !== u.slots.map((_, k) => k).join() || /[⟦⟧]/.test(out.replace(TOKEN, ""))) throw 0;
    return replay(u, u.lead + out.trim() + u.trail, get);
  }

  function replay(u: Unit, s: string, get: (u: Unit) => string): string {
    let out = "";
    let last = 0;
    for (const m of s.matchAll(TOKEN)) {
      out += escape(s.slice(last, m.index), u.syntax, u.pound) + slot(u.slots[+m[1]], get);
      last = m.index + m[0].length;
    }
    return out + escape(s.slice(last), u.syntax, u.pound);
  }

  function slot(sl: Slot, get: (u: Unit) => string): string {
    if (!("options" in sl)) return sl.raw;
    const body = (key: string) => sl.options.find(([k]) => k === key)?.[1];
    let keys = sl.options.map(([k]) => k);
    if (sl.kind !== "select") {
      // Target-locale CLDR categories (Intl), exact `=N` kept; missing categories reuse `other`.
      const cats = new Intl.PluralRules(opts.locale, { type: sl.kind === "plural" ? "cardinal" : "ordinal" })
        .resolvedOptions().pluralCategories as string[];
      keys = [...keys.filter((k) => k.startsWith("=")), ...cats.filter((c) => c !== "other"), "other"];
    }
    const parts = keys.map((k) => {
      const b = body(k) ?? body("other");
      if (!b) throw 0;
      return `${k} {${render(unitOf.get(b)!, get)}}`;
    });
    return `${sl.head}${parts.join(" ")}}`;
  }

  return {
    units,
    restore(get) {
      try { return render(top, get); } catch { return null; }
    },
  };
}

/** i18next plural keys: `x_other` → add the target locale's missing `x_few`/`x_many`… (copy of `x_other`). */
export function i18nextPluralKeys(flat: Record<string, string>, locale: string): Record<string, string> {
  const cats = new Intl.PluralRules(locale).resolvedOptions().pluralCategories as string[];
  const out = { ...flat };
  for (const [k, v] of Object.entries(flat)) {
    const m = /^(.*)_other$/.exec(k);
    if (!m) continue;
    for (const c of cats) out[`${m[1]}_${c}`] ??= v;
  }
  return out;
}
