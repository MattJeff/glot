import { test } from "node:test";
import assert from "node:assert/strict";
import { readXpHeader, withExperiments, writeXpHeader } from "../src/experiments.ts";

const m = { home: { hero: { title: "Ship faster", cta: "Start" } }, x: "y" };
const a = [{ id: "e1", cell: "*", arm: 1 }];

test("x-glot-xp round-trips (UTF-8, base64url without padding) and garbage reads as null", () => {
  const h = writeXpHeader({ v: 1, locale: "de", a, o: { "home.hero.title": "Schneller liefern — für 2 000 Teams" } });
  assert.doesNotMatch(h, /[+/=]/);
  assert.deepEqual(readXpHeader(h)?.o, { "home.hero.title": "Schneller liefern — für 2 000 Teams" });
  for (const bad of [null, "", "%%%", writeXpHeader({ v: 2, locale: "de", a } as never)]) assert.equal(readXpHeader(bad), null);
});

test("withExperiments applies the overlay for the served locale only", () => {
  const header = writeXpHeader({ v: 1, locale: "en", a, o: { "home.hero.title": "Join 2,000 teams", "nope.key": "z" } });
  const out = withExperiments(m, { header, locale: "en" });
  assert.equal(out.home.hero.title, "Join 2,000 teams");
  assert.equal(out.home.hero.cta, "Start");
  assert.equal(m.home.hero.title, "Ship faster", "not mutated");
  assert.equal(withExperiments(m, { header, locale: "de" }), m, "edge locale ≠ served locale → official");
  assert.equal(withExperiments(m, { header: null }), m);
  assert.equal(withExperiments(m, { header: writeXpHeader({ v: 1, locale: "en", a: [{ id: "e1", cell: "*", arm: 0 }] }) }), m, "arm 0");
});

test("withExperiments rebuilds the overlay from the config when the edge sent assignments only", () => {
  const config = {
    experiments: [
      { id: "e1", locales: ["en", "de"], arms: [{ arm: 0 }, { arm: 1, values: { en: { "home.hero.cta": "Try it free" } } }] },
      { id: "e2", locales: ["de"], arms: [{ arm: 0 }, { arm: 1, values: { en: { "home.hero.title": "not this locale" } } }] },
    ],
  };
  const header = writeXpHeader({ v: 1, locale: "en", a: [...a, { id: "e2", cell: "*", arm: 1 }] });
  const out = withExperiments(m, { header, config });
  assert.deepEqual(out.home.hero, { title: "Ship faster", cta: "Try it free" });
});

test("observeSeen: ≥ 50 % visible for ≥ 1 s = seen, cumulative time bucketed, reported once on hide", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
  const { observeSeen, bucket } = await import("../src/experiments.ts");
  const et = new EventTarget();
  let cb: (e: { target: unknown; isIntersecting: boolean; intersectionRatio: number }[]) => void = () => {};
  const hero = { dataset: { glotKey: "home.hero.title" } }, other = { dataset: { glotKey: "footer" } };
  const doc = { visibilityState: "visible", body: {}, querySelectorAll: () => [hero, other] };
  const g = {
    document: doc,
    IntersectionObserver: class { constructor(f: typeof cb) { cb = f; } observe() {} disconnect() {} },
    addEventListener: et.addEventListener.bind(et),
    removeEventListener: et.removeEventListener.bind(et),
  };
  for (const [k, value] of Object.entries(g)) Object.defineProperty(globalThis, k, { value, configurable: true });
  const out: unknown[] = [];
  const keys = { e1: ["home.hero.title", "home.hero.cta"], e2: ["pricing.cta"] };
  const stop = observeSeen([{ id: "e1", cell: "*", arm: 1 }, { id: "e2", cell: "*", arm: 0 }, { id: "hold", cell: "*", arm: 1 }], keys, (s) => out.push(s));
  const at = (ratio: number) => cb([{ target: hero, isIntersecting: ratio > 0, intersectionRatio: ratio }]);

  at(0.4); t.mock.timers.tick(5000); // under 50 %: nothing
  at(0.6); t.mock.timers.tick(800); at(0); // < 1 s continuous: not seen
  et.dispatchEvent(new Event("visibilitychange"));
  assert.deepEqual(out, []);
  at(1); t.mock.timers.tick(1500); at(0); // seen; total 800 + 1500 ms
  at(1); t.mock.timers.tick(3000);
  doc.visibilityState = "hidden";
  et.dispatchEvent(new Event("visibilitychange")); // 5300 ms → bucket 5000
  et.dispatchEvent(new Event("pagehide"));
  stop();
  assert.deepEqual(out, [{ id: "e1", cell: "*", arm: 1, seen: true, visible_ms: 5000 }]);
  assert.deepEqual([bucket(999), bucket(1000), bucket(29_999), bucket(1e9)], [0, 1000, 10000, 60000]);
});
