// @glot/insights/layout — Layout RUM: which translated strings really overflow their box for real visitors.
// Opt-in, sampled (1 % of page views by default), no cookie, no id, no text, no URL: only
// key × locale × rounded viewport × box/needed px × font family (and whether that webfont had loaded).
export type LayoutOptions = {
  /** Project public key (dashboard → Insights). */
  key: string;
  endpoint?: string;
  /** Locale the app rendered (default: <html lang>). */
  locale?: string;
  /** Share of page views measured, 0..1 (default 0.01). */
  sample?: number;
  /** Send after this many ms if the page is still open (default 10 s). */
  delay?: number;
};
type Item = { k: string; o: 0 | 1; b: number; n: number; f: string; fs: number; fl: 0 | 1 };

const SEL = "[data-glot-key],[data-glot-copy]";

/** Overflow of one element: [overflows, box px, needed px]. Ellipsis, clip, line-clamp and text spilling
 * out of its box (overlapping neighbours) all show up as scroll size > client size. */
export function measure(el: HTMLElement): [boolean, number, number] {
  const w = el.scrollWidth > el.clientWidth + 1;
  const h = el.scrollHeight > el.clientHeight + 1 && getComputedStyle(el).overflowY != "visible";
  return [w || h, el.clientWidth, w ? el.scrollWidth : Math.round((el.clientWidth * el.scrollHeight) / (el.clientHeight || 1))];
}

/** Starts measuring this page view (if sampled in); returns a stop function. */
export function watchLayout(o: LayoutOptions): () => void {
  const nav = navigator as Navigator & { globalPrivacyControl?: boolean; webdriver?: boolean };
  if (nav.globalPrivacyControl || nav.doNotTrack == "1" || nav.webdriver || /bot|crawl|spider|headless|lighthouse/i.test(nav.userAgent || "") || Math.random() >= (o.sample ?? 0.01))
    return () => {};
  const seen = new Map<string, Item>();
  const idle = (f: () => void) => ((globalThis as { requestIdleCallback?: (f: () => void) => void }).requestIdleCallback || setTimeout)(f);
  let queued = false;
  const scan = () => {
    queued = false;
    for (const el of document.querySelectorAll<HTMLElement>(SEL)) {
      const k = el.dataset.glotKey || el.dataset.glotCopy;
      if (!k || !el.clientWidth) continue; // hidden: nothing to judge
      ro?.observe(el);
      const [over, b, n] = measure(el);
      const prev = seen.get(k);
      if (prev?.o) continue; // overflow wins: one view per key per page
      const cs = getComputedStyle(el);
      const f = cs.fontFamily.split(",")[0].replace(/["']/g, "").trim().slice(0, 40);
      const fs = parseFloat(cs.fontSize) || 16;
      seen.set(k, { k: k.slice(0, 200), o: over ? 1 : 0, b, n, f, fs, fl: document.fonts?.check(`${cs.fontWeight} ${fs}px "${f}"`) === false ? 0 : 1 });
    }
  };
  const later = () => !queued && ((queued = true), idle(scan));
  const ro = typeof ResizeObserver == "function" ? new ResizeObserver(later) : undefined;
  let done = false;
  const flush = () => {
    if (done) return;
    done = true;
    clearTimeout(timer);
    ro?.disconnect();
    removeEventListener("visibilitychange", hide);
    scan();
    if (!seen.size) return;
    const body = { locale: o.locale || document.documentElement.lang || "en", vw: Math.round(innerWidth / 10) * 10, items: [...seen.values()].slice(0, 200) };
    void fetch(`${o.endpoint || ""}/api/insights/${o.key}/layout`, { method: "POST", body: JSON.stringify(body), keepalive: true }).catch(() => {});
  };
  const hide = () => document.visibilityState == "hidden" && flush();
  addEventListener("visibilitychange", hide);
  (document.fonts?.ready || Promise.resolve()).then(later);
  const timer = setTimeout(flush, o.delay ?? 10000);
  return flush;
}
