import { test } from "node:test";
import assert from "node:assert/strict";

test("copy variants: stored per session, exposed by key, credited on track, announced once", async () => {
  const store = new Map<string, string>();
  const sent: Record<string, unknown>[] = [];
  let announced = 0;
  const globals = {
    sessionStorage: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => store.set(k, v) },
    navigator: { languages: ["fr-FR"] },
    document: { documentElement: { lang: "fr" } },
    location: { pathname: "/" },
    dispatchEvent: (e: Event) => (announced += e.type === "glot:variants" ? 1 : 0),
    fetch: async (_url: string, init: { body: string }) => {
      sent.push(JSON.parse(init.body));
      return {
        ok: true,
        json: async () => ({ arm: null, locale: null, variants: [{ id: "e1", key: "pricing.cta", arm: 2, value: "Essayez gratuitement" }] }),
      };
    },
  };
  for (const [k, value] of Object.entries(globals)) Object.defineProperty(globalThis, k, { value, configurable: true });
  const { init, track, variant } = await import("../src/index.ts");
  await init({ key: "pk_1" });
  await init({ key: "pk_1" });
  assert.equal(variant("pricing.cta"), "Essayez gratuitement");
  assert.equal(variant("other"), undefined);
  assert.equal(announced, 1);
  assert.equal(sent[0].variants, undefined, "a visit sends no assignment");
  track("signup");
  assert.deepEqual(sent[1].variants, { e1: 2 });
  assert.ok(!JSON.stringify(sent).includes("Essayez"), "the text never goes back to the server");
});
