// Shared E2E probes for glot Edge: hydration errors, painted flashes of source text, no-JS HTML.
import type { Browser, ConsoleMessage } from "playwright";

export interface PageReport {
  url: string;
  /** console.error / uncaught errors (hydration mismatches show up here). */
  errors: string[];
  /** Source strings seen in a painted frame after first paint (rAF sampling = what the user can see). */
  flashes: string[];
  /** Carrying block of the first flashed strings (debugging). */
  samples: string[];
  /** Body text after the app settled. */
  text: string;
  /** Translated strings missing from the final text. */
  missing: string[];
  /** `#prerender` snapshot gone = the SPA mounted (undefined when the page has none). */
  mounted?: boolean;
  lang: string | null;
  canonical: string | null;
}

/**
 * Load `url` in a fresh context and watch every animation frame for `sources` (strings that must never be visible
 * on the localized page); `targets` must be in the final text. `settle` = how long to keep watching (ms), enough
 * for the app to boot, hydrate or re-render. `act` runs after load (e.g. trigger a lazy app mount).
 */
export async function probe(browser: Browser, url: string, o: { sources: string[]; targets: string[]; settle?: number; act?: (p: import("playwright").Page) => Promise<void>; locale?: string }): Promise<PageReport> {
  const ctx = await browser.newContext({ reducedMotion: "reduce", locale: o.locale });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("console", (m: ConsoleMessage) => { if (m.type() === "error") errors.push(m.text().slice(0, 300)); });
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message.slice(0, 300)}`));
  await page.addInitScript((sources: string[]) => {
    const w = window as unknown as { __glotFlashes: string[]; __glotSamples: string[] };
    w.__glotFlashes = [];
    w.__glotSamples = [];
    // a source string is "visible" when a rendered text node is exactly it (a translation that merely contains
    // the source, like glot's offline mock "⟦es⟧ <source>", is not a flash)
    const want = new Set(sources);
    const tick = () => {
      if (document.body) {
        const tw = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        for (let n = tw.nextNode(); n; n = tw.nextNode()) {
          const v = (n.nodeValue ?? "").replace(/\s+/g, " ").trim();
          if (v && want.has(v) && !w.__glotFlashes.includes(v) && n.parentElement?.checkVisibility?.() !== false) {
            w.__glotFlashes.push(v);
            let e = n.parentElement;
            while (e && /^(A|B|I|EM|STRONG|SPAN|BUTTON|LABEL|SMALL|CODE|MARK|SUP|SUB|U|S)$/.test(e.tagName)) e = e.parentElement;
            if (w.__glotSamples.length < 6) w.__glotSamples.push((e ?? n.parentElement)!.outerHTML.slice(0, 2000));
          }
        }
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, o.sources);
  await page.goto(url, { waitUntil: "load" });
  const hadSnapshot = await page.evaluate(() => !!document.getElementById("prerender"));
  await o.act?.(page);
  await page.waitForTimeout(o.settle ?? 1500);
  const { flashes, samples } = await page.evaluate(() => { const w = window as unknown as { __glotFlashes: string[]; __glotSamples: string[] }; return { flashes: w.__glotFlashes, samples: w.__glotSamples }; });
  const text = (await page.evaluate(() => document.body.innerText)).replace(/\s+/g, " ");
  const head = await page.evaluate(() => ({ mounted: !document.getElementById("prerender"), lang: document.documentElement.getAttribute("lang"), canonical: document.querySelector('link[rel="canonical"]')?.getAttribute("href") ?? null }));
  await ctx.close();
  return { url, errors, flashes, samples, text, missing: o.targets.filter((t) => !text.includes(t)), mounted: hadSnapshot ? head.mounted : undefined, lang: head.lang, canonical: head.canonical };
}
