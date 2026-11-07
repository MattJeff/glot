// Shared fixtures with the Rust segmenter: crates/core/tests/fixtures/edge/*.html → *.segments.json.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { segment, segmentHashed, translateHtml, type Segment, type SegmentOpts } from "../src/segment.ts";

const dir = new URL("../../../crates/core/tests/fixtures/edge/", import.meta.url);
// Same options as crates/core/tests/edge_fixtures.rs.
const opts: SegmentOpts = { sourceLocale: "en", skip: [".private"], dnt: ["ClipOnIt"], piiAllow: ["support@cliponit.app"] };
const wire = (segs: Segment[]) =>
  segs.map((s) => ({
    hash: s.hash, text: s.text, role: s.role, ...(s.key !== undefined && { key: s.key }), discoverable: s.discoverable,
    ...(s.values.length && { values: s.values }),
  }));

const files = readdirSync(dir).filter((f) => f.endsWith(".html"));
test("fixtures exist", () => assert.ok(files.length >= 4));
for (const f of files) {
  test(`fixture ${f}: same segments as Rust, identity round trip`, async () => {
    const html = readFileSync(new URL(f, dir), "utf8");
    const want = JSON.parse(readFileSync(new URL(f.replace(/\.html$/, ".segments.json"), dir), "utf8"));
    const got = wire(await segmentHashed(html, opts));
    assert.deepEqual(got, want);
    const same = await translateHtml(html, opts, (s) => s.text);
    assert.deepEqual(wire(await segmentHashed(same, opts)), want);
    assert.equal(await translateHtml(html, opts, () => undefined), html);
  });
}

test("translateHtml: placeholders, attributes, invalid translations keep the source", async () => {
  const html = '<p class="x">Read our <a href="/t" title="Terms page">terms</a>, save 20&nbsp;%.</p><p>Keep</p>';
  const out = await translateHtml(html, {}, (s) =>
    ({
      "Read our <0>terms</0>, save {0} %.": "Lee nuestros <0>términos</0>, ahorra {0} % & más.",
      "Terms page": 'Página "términos"',
      Keep: "<0>broken</0>",
    })[s.text]);
  assert.equal(out, '<p class="x">Lee nuestros <a href="/t" title="Página &quot;términos&quot;">términos</a>, ahorra 20 % &amp; más.</p><p>Keep</p>');
  assert.deepEqual(segment("<p>Hello <span data-glot-user>Jean</span>, hi</p>").map((s) => s.text), ["Hello <0/>, hi"]);
});
