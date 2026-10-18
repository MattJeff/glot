import { test } from "node:test";
import assert from "node:assert/strict";

type El = { dataset: Record<string, string>; scrollWidth: number; clientWidth: number; scrollHeight: number; clientHeight: number };
const el = (key: string, scrollWidth: number, clientWidth: number, attr = "glotKey"): El => ({
  dataset: { [attr]: key },
  scrollWidth,
  clientWidth,
  scrollHeight: 20,
  clientHeight: 20,
});

function setup(nav: Record<string, unknown>, els: El[]) {
  const sent: { url: string; body: any }[] = [];
  const listeners: Record<string, () => void> = {};
  const g = {
    navigator: { userAgent: "Mozilla/5.0", ...nav },
    innerWidth: 374,
    document: {
      documentElement: { lang: "de" },
      visibilityState: "visible",
      querySelectorAll: () => els,
      fonts: { check: (f: string) => !f.includes("Late"), ready: Promise.resolve() },
    },
    getComputedStyle: (e: El) => ({
      overflowY: "hidden",
      fontFamily: e.dataset.glotKey === "late" ? '"Late Sans", sans-serif' : '"Inter", system-ui',
      fontSize: "14px",
      fontWeight: "500",
    }),
    addEventListener: (n: string, f: () => void) => (listeners[n] = f),
    removeEventListener: () => {},
    requestIdleCallback: (f: () => void) => f(),
    fetch: async (url: string, init: { body: string }) => {
      sent.push({ url, body: JSON.parse(init.body) });
      return { ok: true };
    },
  };
  for (const [k, value] of Object.entries(g)) Object.defineProperty(globalThis, k, { value, configurable: true });
  return { sent, listeners };
}

test("reports overflow per key with rounded viewport and font state, once per page view", async () => {
  const { sent } = setup({}, [
    el("invoice.remind", 180, 120),
    el("invoice.remind", 90, 120), // same key elsewhere, fits: overflow wins
    el("nav.home", 60, 120),
    el("late", 50, 60),
    el("copy.cta", 200, 100, "glotCopy"),
    el("hidden", 10, 0),
  ]);
  const { watchLayout } = await import("../src/layout.ts");
  const stop = watchLayout({ key: "pk_1", endpoint: "http://g", sample: 1, delay: 60000 });
  await new Promise((r) => setTimeout(r, 5));
  stop();
  stop();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].url, "http://g/api/insights/pk_1/layout");
  const { locale, vw, items } = sent[0].body;
  assert.equal(locale, "de");
  assert.equal(vw, 370);
  const by = Object.fromEntries(items.map((i: { k: string }) => [i.k, i]));
  assert.deepEqual(Object.keys(by).sort(), ["copy.cta", "invoice.remind", "late", "nav.home"]);
  assert.deepEqual(by["invoice.remind"], { k: "invoice.remind", o: 1, b: 120, n: 180, f: "Inter", fs: 14, fl: 1 });
  assert.equal(by["nav.home"].o, 0);
  assert.equal(by["late"].fl, 0, "webfont not loaded yet");
  assert.equal(by["copy.cta"].o, 1, "runtime <GlotCopy> marker");
  assert.ok(!JSON.stringify(sent).match(/Mozilla|cookie|pathname/));
});

test("GPC, DNT, bots and unsampled views send nothing", async () => {
  const { watchLayout } = await import("../src/layout.ts");
  for (const [nav, sample] of [
    [{ globalPrivacyControl: true }, 1],
    [{ doNotTrack: "1" }, 1],
    [{ webdriver: true }, 1],
    [{ userAgent: "Googlebot/2.1" }, 1],
    [{}, 0],
  ] as const) {
    const { sent } = setup(nav, [el("a", 200, 100)]);
    watchLayout({ key: "pk", sample })();
    await new Promise((r) => setTimeout(r, 1));
    assert.equal(sent.length, 0, JSON.stringify(nav));
  }
});

test("flushes when the page is hidden", async () => {
  const { sent, listeners } = setup({}, [el("a", 200, 100)]);
  const { watchLayout } = await import("../src/layout.ts");
  watchLayout({ key: "pk", sample: 1, delay: 60000 });
  (globalThis as any).document.visibilityState = "hidden";
  listeners.visibilitychange();
  await new Promise((r) => setTimeout(r, 1));
  assert.equal(sent.length, 1);
});
