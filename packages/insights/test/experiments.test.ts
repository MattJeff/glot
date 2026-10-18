import { test } from "node:test";
import assert from "node:assert/strict";

test("track: revenue + segmented exposures (cookie + SSR copy), once per session/experiment/event, none under GPC", async () => {
  const store = new Map<string, string>();
  const sent: Record<string, unknown>[] = [];
  const urls: string[] = [];
  const et = new EventTarget();
  const nav: Record<string, unknown> = { languages: ["de-DE"] };
  const globals = {
    sessionStorage: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => store.set(k, v) },
    navigator: nav,
    document: { documentElement: { lang: "de" }, cookie: "a=1; glot_xp=e1.2.seg-1~hold.1.*" },
    location: { pathname: "/" },
    addEventListener: et.addEventListener.bind(et),
    dispatchEvent: et.dispatchEvent.bind(et),
    fetch: async (url: string, init: { body: string }) => {
      urls.push(url);
      sent.push(JSON.parse(init.body));
      return { ok: true, json: async () => ({}) };
    },
  };
  for (const [k, value] of Object.entries(globals)) Object.defineProperty(globalThis, k, { value, configurable: true });
  // What @glot/runtime's GlotExperiments copies from x-glot-xp (wins over the cookie for the same id).
  store.set("glot_xp", JSON.stringify([{ id: "e1", cell: "seg-1", arm: 1 }, { id: "e3", cell: "*", arm: 0 }]));
  const { init, track } = await import("../src/index.ts");
  await init({ key: "pk_1" });
  assert.equal(sent[0].exposures, undefined, "a visit carries no exposure (the edge counts them)");

  track("purchase", { value: 49, currency: "eur" });
  assert.deepEqual(sent[1].exposures, [
    { id: "e1", cell: "seg-1", arm: 1 },
    { id: "hold", cell: "*", arm: 1 },
    { id: "e3", cell: "*", arm: 0 },
  ]);
  assert.equal(sent[1].value, 49);
  assert.equal(sent[1].currency, "EUR");

  track("purchase", { value: 10, currency: "EUR" });
  assert.equal(sent[2].exposures, undefined, "same event again: already credited this session");
  track("signup");
  assert.equal((sent[3].exposures as unknown[]).length, 3, "another event is credited separately");

  // "Seen" exposure from @glot/runtime: experiments batch, once per session and experiment.
  const seen = { id: "e1", cell: "seg-1", arm: 1, seen: true, visible_ms: 2000 };
  dispatchEvent(new CustomEvent("glot:seen", { detail: seen }));
  dispatchEvent(new CustomEvent("glot:seen", { detail: seen }));
  assert.equal(urls.at(-1), "/api/insights/pk_1/batch");
  assert.deepEqual(sent.at(-1), [{ served: "de", exposures: [seen] }]);
  assert.equal(sent.length, 5, "second glot:seen for e1 not sent");

  store.delete("glot_xp_sent");
  nav.globalPrivacyControl = true;
  dispatchEvent(new CustomEvent("glot:seen", { detail: seen }));
  assert.equal(sent.length, 5, "GPC: no seen beacon");
  track("refund");
  assert.equal(sent[5].exposures, undefined, "GPC: nothing attributed");
});
