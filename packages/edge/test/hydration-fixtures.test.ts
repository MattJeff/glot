// Shared fixtures with hydration.rs: crates/core/tests/fixtures/edge/hydration/*.html (real SvelteKit, Nuxt,
// React Router, SolidStart and Astro pages + scanner edge cases) → *.expected.html, through the same dictionary
// (tm.json). Rust and JS must produce byte-identical pages. UPDATE_FIXTURES=1 rewrites the expected files.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { translatePayloads } from "../src/hydration.ts";

const dir = new URL("../../../crates/core/tests/fixtures/edge/hydration/", import.meta.url);
const tm = JSON.parse(readFileSync(new URL("tm.json", dir), "utf8")) as Record<string, string>;

for (const f of readdirSync(dir).filter((f) => f.endsWith(".html") && !f.endsWith(".expected.html")))
  test(`payload fixture ${f}`, () => {
    const got = translatePayloads(readFileSync(new URL(f, dir), "utf8"), (s) => tm[s]).html;
    const want = new URL(f.replace(/\.html$/, ".expected.html"), dir);
    if (process.env.UPDATE_FIXTURES) writeFileSync(want, got);
    assert.equal(got, readFileSync(want, "utf8"));
  });
