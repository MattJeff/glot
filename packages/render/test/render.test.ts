import { test } from "node:test";
import assert from "node:assert/strict";
import { createClient, fillLocal, GlotRenderError } from "../src/index.ts";

const req = {
  locale: "fr-FR",
  type: "email" as const,
  content: { subject: "Hi {{name}}", html: "<p>Hi {{name}}, total {{total}} <a href=\"{{{url}}}\">pay</a> {{later}}</p>" },
  variables: { name: "Zoë <b>", total: { type: "currency" as const, value: 1234.5, currency: "EUR" }, url: "https://a.io/?x=1&y=2" },
};

test("sends the key, the body and the idempotency key; returns the server's answer", async () => {
  const calls: { url: string; init: RequestInit }[] = [];
  const answer = { id: null, locale: "fr-FR", type: "email", status: "translated", content: { subject: "Salut" }, cache: "hit", units: { total: 1, cached: 1, translated: 0, fallback: 0 }, warnings: [] };
  const glot = createClient({
    key: "glot_rk_test",
    baseUrl: "https://glot.test/",
    fetch: (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify(answer), { status: 200 });
    }) as typeof fetch,
  });
  const r = await glot.render(req, { idempotencyKey: "send-1" });
  assert.deepEqual(r, answer);
  assert.equal(calls[0].url, "https://glot.test/v1/render");
  const h = calls[0].init.headers as Record<string, string>;
  assert.equal(h.authorization, "Bearer glot_rk_test");
  assert.equal(h["idempotency-key"], "send-1");
  assert.equal(JSON.parse(calls[0].init.body as string).variables.name, "Zoë <b>");
});

test("fails open: unreachable server or 5xx returns the source with variables filled, and says so", async () => {
  for (const f of [async () => { throw new TypeError("fetch failed"); }, async () => new Response("{}", { status: 503 })]) {
    const glot = createClient({ key: "k", baseUrl: "https://glot.test", fetch: f as unknown as typeof fetch });
    const r = await glot.render(req);
    assert.equal(r.status, "fallback");
    assert.match(r.fallback_reason ?? "", /^client_fallback:/);
    assert.equal(r.content.subject, "Hi Zoë <b>");
    assert.equal(r.content.html, `<p>Hi Zoë &lt;b&gt;, total ${fillLocal("{{t}}", { t: req.variables.total }, "fr-FR", true)} <a href="https://a.io/?x=1&y=2">pay</a> {{later}}</p>`);
    assert.match(r.content.html ?? "", /1\s234,50\s€/);
    const batch = await glot.renderBatch([req, { locale: "de", type: "text", content: "Hallo {{x}}" }]);
    assert.deepEqual(batch.map((b) => b.status), ["fallback", "fallback"]);
  }
});

test("a refusal that needs a fix throws with the server's message", async () => {
  const glot = createClient({
    key: "k",
    baseUrl: "https://glot.test",
    fetch: (async () => new Response(JSON.stringify({ error: { code: "quota_exceeded", message: "quota reached" } }), { status: 402 })) as unknown as typeof fetch,
  });
  await assert.rejects(glot.render(req), (e: unknown) => e instanceof GlotRenderError && e.status === 402 && e.code === "quota_exceeded" && e.message === "quota reached");
  const strict = createClient({ key: "k", baseUrl: "https://glot.test", failOpen: false, fetch: (async () => { throw new TypeError("down"); }) as unknown as typeof fetch });
  await assert.rejects(strict.render(req), /down/);
});
