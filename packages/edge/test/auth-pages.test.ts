// Pages behind sign-in, end to end in a real Chromium: a test app with an HttpOnly session cookie, a fake glot
// server that records every request it gets, and
//  1. "server" mode: the Cloudflare Worker in front of the app on the same host (`/es/dashboard`) translates the
//     signed-in page per response (Cache-Control: private, Vary: Cookie, never in the shared cache, no discovery),
//     the session cookie reaches the app (page + its own `/api/me` fetch) and the client-rendered UI is translated;
//  2. "patcher" mode: one `<script data-glot-app>` tag on the app's own page, the visitor picks Spanish in the
//     switcher, UI translated in place; discovery (opt-in) sends `[data-glot-ui]` text + the hash of a known text,
//     nothing else; off by default.
// In every case: no email, name, amount or cookie of the user ever reaches glot.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type Server } from "node:http";
import { existsSync } from "node:fs";
import { buildSync } from "esbuild";
import { hash16 } from "../src/segment.ts";

const PW = new URL("../../renderer/node_modules/playwright/index.mjs", import.meta.url).pathname;
const noBrowser = !existsSync(PW) && "playwright (packages/renderer) not installed";
const ROOT = new URL("..", import.meta.url).pathname;
const iife = (entry: string) => buildSync({ entryPoints: [ROOT + entry], bundle: true, write: false, format: "iife", target: "es2022" }).outputFiles[0].text;
const h = (t: string) => hash16(`en\u0000${t}`);
const USER = { name: "Jane Doe", email: "jane.doe@example.com", balance: "1,234.56" };
const SECRET = "s3cr3t-session-value";

const listen = (s: Server, host: string) => new Promise<number>((ok) => s.listen(0, host, () => ok((s.address() as { port: number }).port)));
const body = (req: IncomingMessage) => new Promise<string>((ok) => { let b = ""; req.on("data", (c) => (b += c)).on("end", () => ok(b)); });

/** The app: /login sets the session, /dashboard and /api/me need it. `?app=1` = page carrying the glot-app tag. */
function app(glot: () => string, seen: string[]) {
  return createServer((req, res) => {
    const u = new URL(req.url!, "http://x");
    const signedIn = (req.headers.cookie ?? "").includes(`app_session=${SECRET}`);
    seen.push(`${u.pathname} ${signedIn ? "session" : "anon"}`);
    if (u.pathname === "/login") return res.writeHead(302, { location: "/dashboard" + u.search, "set-cookie": `app_session=${SECRET}; HttpOnly; Path=/; SameSite=Lax` }).end();
    if (u.pathname === "/") return res.writeHead(200, { "content-type": "text/html", "cache-control": "public, max-age=60" }).end("<!doctype html><html lang=en><head><title>Home</title></head><body><h1>Welcome back</h1></body></html>");
    if (!signedIn) return res.writeHead(401).end();
    if (u.pathname === "/api/me") return res.writeHead(200, { "content-type": "application/json", "cache-control": "private" }).end(JSON.stringify(USER));
    const tag = u.searchParams.has("app") ? `<script async src="${glot()}/widget/glot-app.global.js" data-glot-app data-project="site" data-server="${glot()}" data-source="en" data-locales="es" data-key="pk_test"></script>` : "";
    // Worst case: the app forgets to mark its dashboard private. glot must not cache it anyway.
    res.writeHead(200, { "content-type": "text/html; charset=utf-8", "cache-control": "public, max-age=60" }).end(`<!doctype html><html lang="en"><head><title>Dashboard</title>${tag}</head><body>
<nav><a href="/dashboard">Dashboard</a> <a href="/settings">Settings</a> <span data-glot-user>${USER.name}</span></nav>
<h1>Welcome back</h1><p class="me">Signed in as ${USER.email}</p>
<section><h2 data-glot-ui>Billing history</h2><table><tr><th>Amount</th></tr><tr><td>€${USER.balance}</td></tr></table></section>
<button>Sign out</button><div id="client"></div>
<script>fetch("/api/me").then((r) => r.json()).then((u) => { document.getElementById("client").innerHTML = "<h3>" + u.name + "</h3><button>Export invoices</button><p>Balance: €" + u.balance + "</p>"; });</script>
</body></html>`);
  });
}

const TM = {
  v: 5, locale: "es",
  t: {
    [h("Dashboard")]: "Panel", [h("Welcome back")]: "Hola de nuevo", [h("Signed in as {0}")]: "Conectado como {0}",
    [h("Amount")]: "Importe", [h("Sign out")]: "Cerrar sesión", [h("Export invoices")]: "Exportar facturas", [h("Balance: €{0}")]: "Saldo: {0} €",
    [h("Settings")]: { "button|/account": "Ajustes" }, // known to glot, not for a link on /dashboard → hash-only candidate
  },
};

function glotServer(auth: { pages: string; discover: boolean }, log: { url: string; body: string; cookie?: string }[]) {
  const js = { "/edge/patcher.js": iife("src/patcher.global.ts"), "/widget/glot-app.global.js": iife("src/app.global.ts") };
  return createServer(async (req, res) => {
    const u = new URL(req.url!, "http://x");
    const b = await body(req);
    log.push({ url: req.url!, body: b, cookie: req.headers.cookie });
    const cors = { "access-control-allow-origin": "*" };
    if (u.pathname in js) return res.writeHead(200, { "content-type": "text/javascript", ...cors }).end(js[u.pathname as keyof typeof js]);
    if (u.pathname === "/edge/v1/site/status") return res.writeHead(200, { "content-type": "application/json", ...cors }).end(JSON.stringify({ enabled: true, off: [], versions: { es: 5 }, auth }));
    if (u.pathname === "/edge/v1/site/es/tm") return res.writeHead(200, { "content-type": "application/json", ...cors }).end(JSON.stringify(TM));
    if (u.pathname === "/edge/v1/site/missing") return res.writeHead(202, cors).end("{}");
    res.writeHead(404, cors).end("{}");
  });
}

/** Nothing about the user in anything glot received (URLs, bodies, cookies). */
function assertNothingPersonal(log: { url: string; body: string; cookie?: string }[]) {
  const all = JSON.stringify(log);
  for (const x of [USER.name, "Jane", USER.email, "jane.doe", USER.balance, "1234", SECRET, "app_session"]) assert.ok(!all.includes(x), `glot received "${x}": ${all}`);
}

test("signed-in pages: Worker server mode (private, uncached) and app-mode patcher, nothing personal to glot", { skip: noBrowser }, async () => {
  const { chromium } = await import(PW);
  const b = await chromium.launch();
  const servers: Server[] = [];
  try {
    // ---------------------------------------------------------------- 1. server mode, Worker on the app's host
    const glotLog: { url: string; body: string; cookie?: string }[] = [], appSeen: string[] = [];
    const g = glotServer({ pages: "server", discover: false }, glotLog);
    const gPort = await listen(g, "127.0.0.1");
    const glot = `http://127.0.0.1:${gPort}`;
    const a = app(() => glot, appSeen);
    const aPort = await listen(a, "127.0.0.1");
    const puts: string[] = [];
    (globalThis as any).caches = { default: { match: async () => undefined, put: async (k: string) => void (k.includes("__glot=") && puts.push(k)) } };
    const { default: worker } = await import("../src/worker.ts");
    const env = { GLOT_SERVER: glot, GLOT_PROJECT: "site", LOCALES: "es", GLOT_KEY: "pk_test" };
    // The zone: the browser talks to this host; the Worker's `fetch(req)` reaches the app (origin) on its own port.
    const front = createServer(async (req, res) => {
      const headers = new Headers();
      for (const [k, v] of Object.entries(req.headers)) if (typeof v === "string") headers.set(k, v);
      const r = new Request(`http://127.0.0.1:${aPort}${req.url}`, { method: req.method, headers, body: req.method === "POST" ? await body(req) : undefined, redirect: "manual" });
      const bg: Promise<unknown>[] = [];
      const out = await worker.fetch(r, env, { waitUntil: (p) => void bg.push(p) });
      const hs: Record<string, string | string[]> = {};
      out.headers.forEach((v, k) => (hs[k] = v));
      if (out.headers.getSetCookie().length) hs["set-cookie"] = out.headers.getSetCookie();
      res.writeHead(out.status, hs).end(Buffer.from(await out.arrayBuffer()));
      await Promise.all(bg);
    });
    servers.push(g, a, front);
    const fPort = await listen(front, "localhost");
    const site = `http://localhost:${fPort}`;

    // An anonymous public page is shared-cached (control)…
    const anon = await fetch(`${site}/es/`, { headers: { accept: "text/html" } });
    assert.match(await anon.text(), /Hola de nuevo/);
    await new Promise((r) => setTimeout(r, 50));
    assert.equal(puts.length, 1, "anonymous page cached");
    const before = glotLog.length; // the anonymous page reported its segments: fine, it's public

    const ctx = await b.newContext({ locale: "en-US" });
    const page = await ctx.newPage();
    const errors: string[] = [];
    page.on("pageerror", (e: Error) => errors.push(e.message));
    await page.goto(`${site}/login`); // → /dashboard (source), cookie set for the site's host
    const resp = await page.goto(`${site}/es/dashboard`);
    assert.equal(resp.headers()["cache-control"], "private, no-cache");
    assert.match(resp.headers()["vary"] ?? "", /Cookie/i);
    assert.equal(resp.headers()["x-glot-edge"], "translated");
    await page.waitForFunction(() => document.querySelector("#client button")?.textContent === "Exportar facturas", null, { timeout: 5000 });
    const text = await page.evaluate(() => document.body.innerText);
    for (const s of ["Panel", "Hola de nuevo", `Conectado como ${USER.email}`, "Importe", "Cerrar sesión", `${USER.name}`, `Saldo: ${USER.balance} €`]) assert.ok(text.includes(s), `${s} in ${text}`);
    assert.equal(puts.length, 1, "signed-in page never put in the shared cache");
    assert.ok(appSeen.includes("/dashboard session") && appSeen.includes("/api/me session"), `session kept: ${appSeen}`);
    const cookies = await ctx.cookies(site);
    assert.equal(cookies.find((c: { name: string }) => c.name === "app_session")?.value, SECRET);
    assert.ok(!glotLog.slice(before).some((l) => l.url.includes("/missing")), "no discovery behind sign-in");
    assertNothingPersonal(glotLog);
    assert.deepEqual(errors, []);
    await ctx.close();

    // ---------------------------------------------------------------- 2. patcher (app) mode, no proxy at all
    for (const discover of [false, true]) {
      const log: { url: string; body: string; cookie?: string }[] = [], seen: string[] = [];
      const g2 = glotServer({ pages: "patcher", discover }, log);
      const glot2 = `http://127.0.0.1:${await listen(g2, "127.0.0.1")}`;
      const a2 = app(() => glot2, seen);
      servers.push(g2, a2);
      const origin = `http://localhost:${await listen(a2, "localhost")}`;
      const c2 = await b.newContext({ locale: "en-US" });
      const p2 = await c2.newPage();
      await p2.goto(`${origin}/login?app=1`);
      await p2.waitForSelector("select[aria-label=Language]");
      assert.equal(await p2.textContent("h1"), "Welcome back"); // the visitor's language is the source: untouched
      await Promise.all([p2.waitForNavigation(), p2.selectOption("select[aria-label=Language]", "es")]);
      await p2.waitForFunction(() => document.querySelector("#client button")?.textContent === "Exportar facturas", null, { timeout: 5000 });
      const t2 = await p2.evaluate(() => document.body.innerText);
      for (const s of ["Panel", "Hola de nuevo", `Conectado como ${USER.email}`, "Cerrar sesión", USER.name, `Saldo: ${USER.balance} €`]) assert.ok(t2.includes(s), `${s} in ${t2}`);
      assert.equal(await p2.evaluate(() => document.documentElement.lang), "es");
      assert.ok(seen.includes("/api/me session"), "the app's own session still works");
      await p2.waitForTimeout(2000); // discovery debounce
      const posts = log.filter((l) => l.url.includes("/missing")).map((l) => JSON.parse(l.body));
      if (!discover) assert.deepEqual(posts, [], "discovery off by default");
      else {
        assert.equal(posts.length, 1, JSON.stringify(posts));
        assert.equal(posts[0].mode, "app");
        assert.equal(posts[0].auth, true);
        assert.equal(posts[0].key, "pk_test");
        const segs = posts[0].segments.map((s: { hash: string; text: string; role: string }) => [s.text, s.hash, s.role]).sort();
        // `[data-glot-ui]` text as text; "Settings" (known to glot, other context) as its hash alone; nothing else
        // ("Jane Doe" in an <h3> is a UI candidate but unknown to glot: never sent, not even hashed).
        assert.deepEqual(segs, [["", h("Settings"), "a"], ["Billing history", h("Billing history"), "h2"]]);
      }
      assert.ok(log.every((l) => !l.cookie), "no cookie ever sent to glot");
      assertNothingPersonal(log);
      await c2.close();
    }

    // ---------------------------------------------------------------- 3. project not in patcher mode: tag inert
    const log3: { url: string; body: string }[] = [];
    const g3 = glotServer({ pages: "server", discover: true }, log3);
    const glot3 = `http://127.0.0.1:${await listen(g3, "127.0.0.1")}`;
    const a3 = app(() => glot3, []);
    servers.push(g3, a3);
    const c3 = await b.newContext({ locale: "es-ES" }); // browser language picked by default
    const p3 = await c3.newPage();
    await p3.goto(`http://localhost:${await listen(a3, "localhost")}/login?app=1`);
    await p3.waitForSelector("select[aria-label=Language]");
    await p3.waitForTimeout(500);
    assert.equal(await p3.textContent("h1"), "Welcome back");
    assert.ok(!log3.some((l) => l.url.includes("/tm")), "no TM fetched");
    await c3.close();
  } finally {
    await b.close();
    for (const s of servers) s.close();
  }
});

test("Next.js proxy, signed-in /es/dashboard: the app gets the cookie, its Set-Cookie comes back, private + translated, nothing reported", async () => {
  const { glotProxy } = await import("../src/next-proxy.ts");
  const { NextRequest } = await import("next/server.js");
  const glotLog: { url: string; body: string }[] = [];
  const glot = (async (input: string, init?: RequestInit) => {
    const u = String(input);
    glotLog.push({ url: u, body: String(init?.body ?? "") });
    if (u.endsWith("/status")) return new Response(JSON.stringify({ enabled: true, off: [], versions: { es: 5 }, auth: { pages: "server" } }));
    if (u.endsWith("/es/tm")) return new Response(JSON.stringify(TM));
    return new Response("{}");
  }) as unknown as typeof fetch;
  let cookie: string | null = null;
  const saved = globalThis.fetch;
  globalThis.fetch = (async (_: URL | string, init?: RequestInit) => {
    cookie = new Headers(init?.headers).get("cookie");
    return new Response(`<html><head></head><body><h1>Welcome back</h1><p>Signed in as ${USER.email}</p></body></html>`, { headers: { "content-type": "text/html", "cache-control": "public, max-age=60", "set-cookie": "app_session=rotated; HttpOnly; Path=/" } });
  }) as typeof fetch;
  try {
    const proxy = glotProxy({ source: "en", locales: ["es"], edge: { server: "https://glot", project: "site", key: "pk_test", fetch: glot } });
    const res = await proxy(new NextRequest("https://cliponit.test/es/dashboard", { headers: { accept: "text/html", cookie: `app_session=${SECRET}` } }));
    assert.equal(cookie, `app_session=${SECRET}`);
    assert.equal(res.headers.get("set-cookie"), "app_session=rotated; HttpOnly; Path=/");
    assert.equal(res.headers.get("cache-control"), "private, no-cache");
    assert.match(res.headers.get("vary")!, /Cookie/);
    const html = await res.text();
    assert.match(html, /<h1>Hola de nuevo<\/h1>/);
    assert.match(html, new RegExp(`Conectado como ${USER.email}`));
    assert.doesNotMatch(html, /data-missing/); // the patcher tag never reports from a signed-in page
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(!glotLog.some((l) => l.url.includes("/missing")));
    assertNothingPersonal(glotLog);
  } finally {
    globalThis.fetch = saved;
  }
});
