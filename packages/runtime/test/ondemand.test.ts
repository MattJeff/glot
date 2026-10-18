// createOnDemandLoader against a fake glot CDN: 202 → null, then the catalog (merged over the source), ETag reuse.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createOnDemandLoader, acceptedLanguages } from "../src/ondemand.ts";

test("202 then catalog, cached, revalidated with ETag", async () => {
  let ready = false;
  const calls: { url: string; etag?: string }[] = [];
  const fake = (async (url: string, init?: RequestInit) => {
    const etag = (init?.headers as Record<string, string>)?.["if-none-match"];
    calls.push({ url, etag });
    if (!ready) return new Response('{"status":"translating"}', { status: 202, headers: { "retry-after": "0" } });
    if (etag === '"v1"') return new Response(null, { status: 304 });
    return new Response('{"home":{"title":"Willkommen"}}', { status: 200, headers: { etag: '"v1"' } });
  }) as typeof fetch;
  const load = createOnDemandLoader({ server: "http://glot/", project: "demo", source: { home: { title: "Welcome", cta: "Go" } }, ttl: 0, fetch: fake });

  const [a, b] = await Promise.all([load("de"), load("de")]);
  assert.equal(a, null);
  assert.equal(b, null);
  assert.equal(calls.length, 1, "concurrent first requests share one fetch");
  assert.equal(calls[0].url, "http://glot/cdn/demo/de.json?ondemand=1");

  ready = true;
  assert.deepEqual(await load("de"), { home: { title: "Willkommen", cta: "Go" } }); // pending: awaited, not background
  assert.deepEqual(await load("de"), { home: { title: "Willkommen", cta: "Go" } }); // stale: served, revalidated
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(calls.at(-1)?.etag, '"v1"');
});

test("glot unreachable: null, never throws", async () => {
  const load = createOnDemandLoader({ server: "http://glot", project: "p", fetch: (async () => { throw new Error("down"); }) as typeof fetch });
  assert.equal(await load("de"), null);
});

test("Accept-Language order", () => {
  assert.deepEqual(acceptedLanguages("fr;q=0.5, de-DE, *;q=0.1, en;q=0"), ["de-DE", "fr"]);
  assert.deepEqual(acceptedLanguages(null), []);
});
