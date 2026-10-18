// Independent judge for glot-bench: FormatJS's ICU parser + the JS engine's own CLDR plural rules
// (Intl.PluralRules), so glot's Rust validator never grades its own output alone.
// In: JSONL {id, locale, source, target} (file arg or stdin). Out: JSONL {id, issues: [code...]}.
import { readFileSync } from "node:fs";
import { parse, TYPE } from "@formatjs/icu-messageformat-parser";

const NAMED = new Set([TYPE.argument, TYPE.number, TYPE.date, TYPE.time, TYPE.select, TYPE.plural]);

function walk(nodes, f) {
  for (const n of nodes) {
    f(n);
    if (n.options) for (const o of Object.values(n.options)) walk(o.value, f);
    if (n.type === TYPE.tag) walk(n.children, f);
  }
}

function facts(ast) {
  const args = new Set(), tags = [], choices = [];
  walk(ast, (n) => {
    if (NAMED.has(n.type)) args.add(n.value);
    if (n.type === TYPE.tag) tags.push(n.value);
    if (n.type === TYPE.select || n.type === TYPE.plural) choices.push(n);
  });
  return { args, tags: tags.sort().join(","), choices };
}

export function judge({ locale, source, target }) {
  if (typeof target !== "string" || !target.trim()) return ["empty"];
  let s, t;
  try { s = facts(parse(source)); } catch { return []; } // unparseable source: nothing to compare
  try { t = facts(parse(target)); } catch { return ["syntax_error"]; }
  const issues = new Set();
  for (const a of s.args) if (!t.args.has(a)) issues.add("placeholder_missing");
  for (const a of t.args) if (!s.args.has(a)) issues.add("placeholder_extra");
  if (s.tags !== t.tags) issues.add("tag_mismatch");
  const cardinal = new Intl.PluralRules(locale).resolvedOptions().pluralCategories;
  for (const c of t.choices) {
    const keys = Object.keys(c.options);
    if (c.type === TYPE.plural && c.pluralType === "cardinal") {
      const missing = cardinal.filter((k) => !keys.includes(k));
      // CLDR `many` in fr/es/it/pt/ca only covers 1 000 000+ (compact numbers): soft, like glot's warning.
      const soft = missing.length === 1 && missing[0] === "many" && ["fr", "es", "it", "pt", "ca"].includes(locale.split(/[-_]/)[0]);
      if (missing.length) issues.add(soft ? "warn:plural_many_missing" : "plural_category_missing");
    } else if (c.type === TYPE.select) {
      const src = s.choices.find((x) => x.value === c.value && x.type === TYPE.select);
      if (src && Object.keys(src.options).sort().join() !== keys.sort().join()) issues.add("select_key_changed");
    }
  }
  return [...issues];
}

const input = readFileSync(process.argv[2] ?? 0, "utf8");
const out = [];
for (const line of input.split("\n")) {
  if (!line.trim()) continue;
  const r = JSON.parse(line);
  out.push(JSON.stringify({ id: r.id, issues: judge(r) }));
}
process.stdout.write(out.join("\n") + (out.length ? "\n" : ""));
