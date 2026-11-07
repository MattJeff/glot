import { test } from "node:test";
import assert from "node:assert/strict";

// Launch holdout: the held-out session renders the fallback; an explicit pick always wins and leaves the arm.
test("held-out session gets the fallback; an explicit pick withdraws it and stops assignments", async () => {
  const session = new Map<string, string>();
  const local = new Map<string, string>();
  const sent: Record<string, unknown>[] = [];
  const events: string[] = [];
  const listeners: Record<string, (e: Event) => void> = {};
  const globals = {
    sessionStorage: { getItem: (k: string) => session.get(k) ?? null, setItem: (k: string, v: string) => session.set(k, v) },
    localStorage: { getItem: (k: string) => local.get(k) ?? null, setItem: (k: string, v: string) => local.set(k, v) },
    navigator: { languages: ["es-MX"] },
    document: { documentElement: { lang: "en" } },
    location: { pathname: "/" },
    addEventListener: (t: string, f: (e: Event) => void) => (listeners[t] = f),
    dispatchEvent: (e: Event) => events.push(`${e.type}:${(e as CustomEvent).detail?.locale ?? ""}`),
    fetch: async (_: string, init: { body: string }) => {
      sent.push(JSON.parse(init.body));
      return { ok: true, json: async () => ({ arm: "control", locale: "en" }) };
    },
  };
  for (const [k, value] of Object.entries(globals)) Object.defineProperty(globalThis, k, { value, configurable: true });
  const { init, track } = await import("../src/index.ts");

  const seen: string[] = [];
  await init({ key: "pk_1", onLocale: (l) => seen.push(l) });
  assert.deepEqual(seen, ["en"], "held out: render the fallback");
  assert.ok(events.includes("glot:locale:en"));
  assert.equal(sent[0].explicit, undefined);

  // The visitor picks Spanish in <glot-locale-switcher> (it stores the choice, then dispatches the event).
  local.set("glot-locale", "es");
  listeners["glot:locale-change"](new Event("glot:locale-change"));
  assert.deepEqual(sent[1], { langs: ["es-MX"], served: "en", arm: "control", explicit: true }, "withdrawn once");
  listeners["glot:locale-change"](new Event("glot:locale-change"));
  assert.equal(sent.length, 2, "no arm left: nothing more to withdraw");

  track("signup");
  assert.equal(sent[2].arm, undefined, "conversions after the pick carry no arm");
  assert.equal(sent[2].explicit, true);

  // Next page of the same session: the explicit choice wins over any stored assignment.
  seen.length = 0;
  await init({ key: "pk_1", onLocale: (l) => seen.push(l) });
  assert.deepEqual(seen, []);
});
