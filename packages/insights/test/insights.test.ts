import { test } from "node:test";
import assert from "node:assert/strict";
import { acceptLanguages } from "../src/server.ts";

test("acceptLanguages orders by q", () => {
  assert.deepEqual(acceptLanguages("en;q=0.5, de-AT, fr;q=0.8, *;q=0.1, x;q=0"), ["de-AT", "fr", "en"]);
  assert.deepEqual(acceptLanguages(null), []);
});

test("init counts one visit per session, applies the experiment, track sends the arm", async () => {
  const store = new Map<string, string>();
  const sent: { url: string; body: Record<string, unknown> }[] = [];
  const globals = {
    sessionStorage: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => store.set(k, v) },
    navigator: { languages: ["de-AT", "en"] },
    document: { documentElement: { lang: "en" } },
    location: { pathname: "/pricing" },
    fetch: async (url: string, init: { body: string }) => {
      sent.push({ url, body: JSON.parse(init.body) });
      return { ok: true, json: async () => ({ arm: "test", locale: "de" }) };
    },
  };
  for (const [k, value] of Object.entries(globals)) Object.defineProperty(globalThis, k, { value, configurable: true });
  const { init, track } = await import("../src/index.ts");
  const seen: string[] = [];
  await init({ key: "pk_1", endpoint: "http://g", onLocale: (l) => seen.push(l) });
  await init({ key: "pk_1", endpoint: "http://g", onLocale: (l) => seen.push(l) });
  track("signup");
  assert.equal(sent.length, 2, "second init is the same session: no new visit");
  assert.equal(sent[0].url, "http://g/api/insights/pk_1");
  assert.deepEqual(sent[0].body, { langs: ["de-AT", "en"], served: "en", route: "/pricing" });
  assert.equal(sent[1].body.event, "signup");
  assert.equal(sent[1].body.arm, "test");
  assert.deepEqual(seen, ["de", "de"]);
  assert.ok(!JSON.stringify(sent).includes("cookie"));
});
