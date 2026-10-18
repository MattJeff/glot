// Masking / restoration, plus the real glot validator (dist/glot.wasm) in Node.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { mask, i18nextPluralKeys, type Glossary } from "../src/mask.ts";

const glossary: Glossary = {
  term: [
    { term: "Workspace", translations: { fr: "Espace de travail", pl: "Obszar roboczy" } },
    { term: "glot", do_not_translate: true },
    { term: "Team", description: "no translation for fr: left to the translator" },
  ],
};

// Fake MT: uppercase letters outside tokens; tokens kept.
const upper = (s: string) => s.replace(/[^⟦⟧\d]+(?=⟦|$)/g, (t) => t.toUpperCase());
const run = (msg: string, locale = "fr", tr = upper, syntax: "icu" | "i18next" | "mf2" = "icu") => {
  const m = mask(msg, { glossary, locale, syntax });
  assert.ok(m, "parsed");
  return { cores: m.units.map((u) => u.core), out: m.restore((u) => tr(u.core)) };
};

test("placeholders, tags and glossary become tokens", () => {
  const { cores, out } = run("Open <b>{name}</b>'s Workspace in glot");
  assert.deepEqual(cores, ["Open ⟦0⟧⟦1⟧⟦2⟧'s ⟦3⟧ in ⟦4⟧"]);
  assert.equal(out, "OPEN <b>{name}</b>''S Espace de travail IN glot");
});

test("leading/trailing whitespace kept, letterless text not translated", () => {
  assert.deepEqual(run("  Hello {n}  ").out, "  HELLO {n}  ");
  const r = run("{count} / {total}");
  assert.deepEqual(r.cores, [""]);
  assert.equal(r.out, "{count} / {total}");
});

test("plural: each branch translated alone, structure kept, target categories added", () => {
  const { cores, out } = run("You have {n, plural, =0 {no files} one {# file} other {# files}} in {dir}.", "pl");
  assert.deepEqual(cores, ["no files", "⟦0⟧ file", "⟦0⟧ files", "You have ⟦0⟧ in ⟦1⟧."]);
  // pl needs one/few/many/other: missing ones reuse `other`.
  assert.equal(out, "YOU HAVE {n, plural, =0 {NO FILES} one {# FILE} few {# FILES} many {# FILES} other {# FILES}} IN {dir}.");
  // ja only has `other`: `one` dropped.
  assert.equal(run("{n, plural, one {# file} other {# files}}", "ja").out, "{n, plural, other {# FILES}}");
});

test("nested select inside plural; quoted literals and apostrophes re-escaped", () => {
  const { out } = run("{g, select, female {She has {n, plural, one {# cat} other {# cats}}} other {It's '{'ok'}'}}");
  assert.equal(out, "{g, select, female {SHE HAS {n, plural, one {# CAT} many {# CATS} other {# CATS}}} other {IT''S '{'OK'}'}}");
});

test("lost, duplicated or invented tokens reject the message", () => {
  assert.equal(run("Hello {name}", "fr", () => "Bonjour").out, null);
  assert.equal(run("Hello {name}", "fr", () => "Bonjour ⟦0⟧ ⟦0⟧").out, null);
  assert.equal(run("Hello {name}", "fr", () => "Bonjour ⟦0⟧ ⟦1⟧").out, null);
  assert.equal(run("Hello {name}", "fr", () => "Bonjour ⟦ 0 ⟧").out, "Bonjour {name}"); // tolerant spacing
});

test("i18next and MF2", () => {
  assert.equal(run("Hi {{name}}, see $t(common.ok) <1>here</1>", "fr", upper, "i18next").out, "HI {{name}}, SEE $t(common.ok) <1>HERE</1>");
  assert.equal(run("Hi {$name}! Price \\{1\\}", "fr", (s) => s.replace("Hi", "Salut") + " {x}", "mf2").out, "Salut {$name}! Price \\{1\\} \\{x\\}");
  assert.equal(mask(".input {$n :number}\n.match $n one {{a}} * {{b}}", { locale: "fr", syntax: "mf2" }), null);
  assert.deepEqual(i18nextPluralKeys({ a_one: "1", a_other: "n" }, "pl"), { a_one: "1", a_other: "n", a_few: "n", a_many: "n" });
});

test("invalid ICU stays source (null)", () => {
  assert.equal(mask("Hello {name", { locale: "fr" }), null);
});

const wasm = new URL("../dist/glot.wasm", import.meta.url);
test("restored output passes glot's validator (WASM)", { skip: !existsSync(wasm) && "run pnpm build:wasm" }, async () => {
  const { loadValidator } = await import("../dist/index.js");
  const check = await loadValidator(readFileSync(wasm));
  const g = (glossary as { term: unknown[] }).term;
  const src = "You have {n, plural, one {# file} other {# files}} in your Workspace";
  const out = run(src, "pl").out!;
  assert.deepEqual(check({ source: src, target: out, locale: "pl", glossary: g }).filter((i: { severity: string }) => i.severity === "error"), []);
  const bad = check({ source: src, target: "{n, plural, other {# pliki}} w Workspace", locale: "pl", glossary: g });
  assert.ok(bad.some((i: { code: string }) => i.code === "plural_category_missing"));
  assert.ok(bad.some((i: { code: string }) => i.code === "glossary_violation"));
});
