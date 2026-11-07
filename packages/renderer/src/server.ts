// glot renderer: headless-Chromium snapshots of SPA pages for glot Edge (`rendering: prerender|auto`).
// Never renders in the request path: GET /snapshot answers from the store (fresh, or stale + background refresh)
// or 202 + queued, and the Edge serves the origin HTML meanwhile (fail-open). Snapshots are the *source* page's
// rendered DOM; Edge translates them at serve time with one frozen TM version, so the title Google sees and the
// one a visitor sees always come from the same version and a TM change needs no re-render (only a deploy does).
//   GET  /snapshot?url=…[&mount=append]   200 html (x-glot-snapshot: fresh|stale) | 202 queued | 400/403
//   POST /refresh {"urls":[…],"mount"?}    re-render now (deploy webhook, sitemap crawl, dashboard "refresh")
//   GET  /metrics                          JSON counters + render p50/p95      GET /healthz
// Auth: `Authorization: Bearer $RENDERER_TOKEN` on everything but /healthz (internal service, glot server only).
// SSRF: http(s) only, no credentials in URLs, the page and every subresource must resolve to public IPs
// (RENDERER_ALLOW_PRIVATE=1 for local tests); images/media/fonts are never fetched.
// ponytail: the IP check runs in Node before Chromium resolves again (DNS-rebinding window); the renderer runs
// on its own network without access to the DB, which caps the blast radius. Pin with --host-resolver-rules if needed.
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { createHash, timingSafeEqual } from "node:crypto";
import { lookup } from "node:dns/promises";
import { BlockList, isIP } from "node:net";
import { mkdir, readFile, writeFile, rename } from "node:fs/promises";
import { join } from "node:path";
import { chromium, type Browser } from "playwright";

const env = process.env;
export const config = {
  port: Number(env.PORT ?? 8795),
  token: env.RENDERER_TOKEN ?? "",
  dir: env.RENDERER_DIR ?? "./snapshots",
  ttlMs: Number(env.RENDERER_TTL_S ?? 86400) * 1000,
  concurrency: Number(env.RENDERER_CONCURRENCY ?? 2),
  timeoutMs: Number(env.RENDERER_TIMEOUT_MS ?? 20000),
  maxBytes: Number(env.RENDERER_MAX_BYTES ?? 5 * 1024 * 1024),
  maxQueue: Number(env.RENDERER_MAX_QUEUE ?? 500),
  allowPrivate: env.RENDERER_ALLOW_PRIVATE === "1",
};

const blocked = new BlockList();
for (const [net, bits] of [["0.0.0.0", 8], ["10.0.0.0", 8], ["100.64.0.0", 10], ["127.0.0.0", 8], ["169.254.0.0", 16], ["172.16.0.0", 12], ["192.0.0.0", 24], ["192.168.0.0", 16], ["198.18.0.0", 15], ["224.0.0.0", 3]] as const) blocked.addSubnet(net, bits, "ipv4");
for (const [net, bits] of [["::", 127], ["fc00::", 7], ["fe80::", 10], ["ff00::", 8]] as const) blocked.addSubnet(net, bits, "ipv6");
const dnsCache = new Map<string, Promise<boolean>>();
/** Every address of `host` is public (cached per process: the renderer only ever sees a handful of hosts). */
export function publicHost(host: string): Promise<boolean> {
  if (config.allowPrivate) return Promise.resolve(true);
  const h = host.replace(/^\[|\]$/g, "");
  let p = dnsCache.get(h);
  if (!p) {
    const check = (a: string) => !blocked.check(a, isIP(a) === 6 ? "ipv6" : "ipv4");
    p = isIP(h) ? Promise.resolve(check(h)) : lookup(h, { all: true }).then((as) => as.length > 0 && as.every((a) => check(a.address)), () => false);
    dnsCache.set(h, p);
  }
  return p;
}

export type Mount = "replace" | "append";
interface Meta { url: string; mount: Mount; renderedAt: number; ms: number }
const key = (url: string, mount: Mount) => createHash("sha256").update(`${mount}\0${url}`).digest("hex").slice(0, 32);

export const metrics = { requests: 0, fresh: 0, stale: 0, misses: 0, queued: 0, rendered: 0, failed: 0, rejected: 0, renderMs: [] as number[] };
const queue: { url: string; mount: Mount }[] = [];
const pending = new Set<string>();
let running = 0;
let browser: Promise<Browser> | undefined;

function enqueue(url: string, mount: Mount): boolean {
  const k = key(url, mount);
  if (pending.has(k)) return true;
  if (queue.length >= config.maxQueue) return false;
  pending.add(k);
  queue.push({ url, mount });
  metrics.queued++;
  void drain();
  return true;
}

async function drain() {
  while (running < config.concurrency && queue.length) {
    const job = queue.shift()!;
    running++;
    renderAndStore(job.url, job.mount)
      .catch((e) => { metrics.failed++; console.error(JSON.stringify({ msg: "render failed", url: job.url, error: String(e?.message ?? e) })); })
      .finally(() => { running--; pending.delete(key(job.url, job.mount)); void drain(); });
  }
}

/** Render `url` in a fresh context, wait for network idle + a quiet DOM (500 ms without mutations), return the DOM. */
export async function render(url: string, mount: Mount = "replace"): Promise<string> {
  browser ??= chromium.launch({ args: ["--disable-dev-shm-usage"] }).catch((e) => { browser = undefined; throw e; });
  const ctx = await (await browser).newContext({ reducedMotion: "reduce", serviceWorkers: "block", userAgent: "Mozilla/5.0 (compatible; glot-renderer/1.0; +https://glotlang.com/docs/edge)" });
  try {
    await ctx.route("**/*", async (route) => {
      const r = route.request();
      const u = new URL(r.url());
      if (u.protocol === "data:" || u.protocol === "blob:") return route.continue();
      if (["image", "media", "font"].includes(r.resourceType()) || !/^https?:$/.test(u.protocol) || !(await publicHost(u.hostname))) return route.abort();
      return route.continue();
    });
    const page = await ctx.newPage();
    const deadline = Date.now() + config.timeoutMs;
    const res = await page.goto(url, { waitUntil: "load", timeout: config.timeoutMs });
    if (!res || res.status() >= 400) throw new Error(`origin answered ${res?.status() ?? "nothing"}`);
    await page.waitForLoadState("networkidle", { timeout: Math.max(1000, deadline - Date.now()) }).catch(() => {});
    await page.evaluate((budget) => new Promise<void>((done) => {
      let t = setTimeout(fin, 500);
      const mo = new MutationObserver(() => { clearTimeout(t); t = setTimeout(fin, 500); });
      const hard = setTimeout(fin, budget);
      function fin() { mo.disconnect(); clearTimeout(hard); done(); }
      mo.observe(document, { subtree: true, childList: true, characterData: true, attributes: true });
    }), Math.max(500, Math.min(5000, deadline - Date.now())));
    // Apps that *append* to their mount point (Leptos mount_to_body, Svelte `target: body`) would duplicate the
    // snapshot: wrap it, the Edge patcher removes the wrapper as soon as the app adds its own first element.
    if (mount === "append") await page.evaluate(() => {
      const w = document.createElement("div");
      w.setAttribute("data-glot-prerender", "");
      w.style.display = "contents";
      for (const n of Array.from(document.body.childNodes)) if (!(n instanceof HTMLScriptElement)) w.appendChild(n);
      document.body.prepend(w);
    });
    const html = "<!DOCTYPE html>\n" + (await page.evaluate(() => document.documentElement.outerHTML));
    if (Buffer.byteLength(html) > config.maxBytes) throw new Error("snapshot too large");
    return html;
  } finally {
    await ctx.close();
  }
}

async function renderAndStore(url: string, mount: Mount) {
  const t0 = performance.now();
  const html = await render(url, mount);
  const ms = Math.round(performance.now() - t0);
  const k = key(url, mount);
  await mkdir(config.dir, { recursive: true });
  // write-then-rename: a reader never sees half a snapshot
  await writeFile(join(config.dir, `${k}.html.tmp`), html);
  await rename(join(config.dir, `${k}.html.tmp`), join(config.dir, `${k}.html`));
  await writeFile(join(config.dir, `${k}.json`), JSON.stringify({ url, mount, renderedAt: Date.now(), ms } satisfies Meta));
  metrics.rendered++;
  metrics.renderMs.push(ms);
  if (metrics.renderMs.length > 500) metrics.renderMs.shift();
}

async function stored(url: string, mount: Mount): Promise<{ html: Buffer; meta: Meta } | null> {
  const k = key(url, mount);
  try {
    const meta = JSON.parse(await readFile(join(config.dir, `${k}.json`), "utf8")) as Meta;
    return { meta, html: await readFile(join(config.dir, `${k}.html`)) };
  } catch {
    return null;
  }
}

/** `url` is fetchable by the renderer: http(s), no userinfo, public host. */
export async function allowed(raw: string | null): Promise<URL | null> {
  if (!raw) return null;
  let u: URL;
  try { u = new URL(raw); } catch { return null; }
  if (!/^https?:$/.test(u.protocol) || u.username || u.password) return null;
  u.hash = "";
  return (await publicHost(u.hostname)) ? u : null;
}

const pct = (xs: number[], p: number) => { if (!xs.length) return null; const s = [...xs].sort((a, b) => a - b); return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))]; };
export function metricsJson() {
  const { renderMs, ...counters } = metrics;
  const served = counters.fresh + counters.stale;
  return { ...counters, queue: queue.length, running, hitRatio: counters.requests ? served / counters.requests : null, renderP50: pct(renderMs, 50), renderP95: pct(renderMs, 95) };
}

function authorized(req: IncomingMessage): boolean {
  if (!config.token) return true;
  const got = Buffer.from(req.headers.authorization ?? "");
  const want = Buffer.from(`Bearer ${config.token}`);
  return got.length === want.length && timingSafeEqual(got, want);
}

const json = (res: ServerResponse, status: number, body: unknown) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(body)); };

export async function handle(req: IncomingMessage, res: ServerResponse) {
  const u = new URL(req.url ?? "/", "http://renderer");
  if (u.pathname === "/healthz") return void res.end("ok");
  if (!authorized(req)) return json(res, 401, { error: "unauthorized" });
  const mount: Mount = u.searchParams.get("mount") === "append" ? "append" : "replace";
  if (req.method === "GET" && u.pathname === "/snapshot") {
    metrics.requests++;
    const target = await allowed(u.searchParams.get("url"));
    if (!target) { metrics.rejected++; return json(res, 403, { error: "url not allowed" }); }
    const s = await stored(target.href, mount);
    if (!s) {
      metrics.misses++;
      return enqueue(target.href, mount) ? json(res, 202, { queued: true }) : json(res, 503, { error: "queue full" });
    }
    const stale = Date.now() - s.meta.renderedAt > config.ttlMs;
    if (stale) { metrics.stale++; enqueue(target.href, mount); } else metrics.fresh++;
    res.writeHead(200, { "content-type": "text/html; charset=utf-8", "x-glot-snapshot": stale ? "stale" : "fresh", "x-glot-rendered-at": new Date(s.meta.renderedAt).toISOString() });
    return void res.end(s.html);
  }
  if (req.method === "POST" && u.pathname === "/refresh") {
    let body = "";
    for await (const c of req) { body += c; if (body.length > 1 << 20) return json(res, 413, { error: "too large" }); }
    let urls: unknown;
    let m: Mount = mount;
    try { const b = JSON.parse(body); urls = b.urls; if (b.mount === "append") m = "append"; } catch { return json(res, 400, { error: "json" }); }
    if (!Array.isArray(urls)) return json(res, 400, { error: "urls[] required" });
    let queued = 0;
    for (const raw of urls.slice(0, 1000)) {
      const t = await allowed(String(raw));
      if (t && enqueue(t.href, m)) queued++;
    }
    return json(res, 202, { queued });
  }
  if (req.method === "GET" && u.pathname === "/metrics") return json(res, 200, metricsJson());
  json(res, 404, { error: "not found" });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  if (!config.token && env.NODE_ENV === "production") { console.error("RENDERER_TOKEN is required (GLOT_RENDERER_TOKEN in deploy/.env)"); process.exit(1); }
  const srv = createServer((req, res) => void handle(req, res).catch((e) => { console.error(e); if (!res.headersSent) json(res, 500, { error: "internal" }); }));
  srv.listen(config.port, () => console.log(JSON.stringify({ msg: "glot renderer listening", port: config.port, dir: config.dir })));
  const stop = async () => { srv.close(); await (await browser)?.close().catch(() => {}); process.exit(0); };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
}
