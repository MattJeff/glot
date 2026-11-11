import { test, after } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isSkeleton } from "../src/skeleton.ts";

process.env.RENDERER_DIR = mkdtempSync(join(tmpdir(), "glot-renderer-"));
process.env.RENDERER_TOKEN = "t0k";
const { config, handle, publicHost, metricsJson, render, blockedRequest, stopBrowser } = await import("../src/server.ts");
const { prerender, routeFile } = await import("../src/cli.ts");

// a CSR app: empty shell, content rendered by JS after a fetch
const SHELL = `<!DOCTYPE html><html><head><title>App</title></head><body><div id="root"></div><script>
fetch("/data").then(r => r.json()).then(d => { document.getElementById("root").innerHTML = "<h1>" + d.title + "</h1><p>" + "Long enough paragraph. ".repeat(12) + "</p>"; });
</script></body></html>`;
const origin: Server = createServer((req, res) => {
  if (req.url === "/hang") { res.setHeader("content-type", "text/html"); return res.end("<html><body><p>x</p><script>for(;;){}</script></body></html>"); }
  if (req.url === "/data") return res.end(JSON.stringify({ title: "Rendered by the browser" }));
  res.setHeader("content-type", "text/html");
  res.end(SHELL);
});
await new Promise<void>((r) => origin.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${(origin.address() as { port: number }).port}`;
const api: Server = createServer((q, s) => void handle(q, s));
await new Promise<void>((r) => api.listen(0, "127.0.0.1", r));
const rbase = `http://127.0.0.1:${(api.address() as { port: number }).port}`;
const auth = { authorization: "Bearer t0k" };
after(async () => { await stopBrowser(); origin.close(); api.close(); setTimeout(() => process.exit(0), 50); });

test("skeleton heuristic", () => {
  assert.equal(isSkeleton(SHELL), true);
  assert.equal(isSkeleton(`<body><h1>Hi</h1><p>${"word ".repeat(60)}</p><script>${"x".repeat(5000)}</script></body>`), false);
  assert.equal(isSkeleton(`<body><div>${"word ".repeat(60)}</div></body>`), true, "text but no content element");
});

test("private hosts are refused unless allowed", async () => {
  assert.equal(await publicHost("127.0.0.1"), false);
  assert.equal(await publicHost("10.1.2.3"), false);
  assert.equal(await publicHost("::1"), false);
  assert.equal(await publicHost("1.1.1.1"), true);
  const r = await fetch(`${rbase}/snapshot?url=${encodeURIComponent(base + "/")}`, { headers: auth });
  assert.equal(r.status, 403);
});

test("auth required", async () => {
  assert.equal((await fetch(`${rbase}/metrics`)).status, 401);
  assert.equal((await fetch(`${rbase}/healthz`)).status, 200);
});

test("miss → 202 queued, never rendered in the request; then served from the store", async () => {
  config.allowPrivate = true;
  const url = `${rbase}/snapshot?url=${encodeURIComponent(base + "/pricing")}`;
  const t0 = performance.now();
  const first = await fetch(url, { headers: auth });
  assert.equal(first.status, 202);
  assert.ok(performance.now() - t0 < 500, "the miss answers immediately");
  let html = "";
  for (let i = 0; i < 100 && !html; i++) {
    await new Promise((r) => setTimeout(r, 200));
    const r = await fetch(url, { headers: auth });
    if (r.status === 200) { assert.equal(r.headers.get("x-glot-snapshot"), "fresh"); html = await r.text(); }
  }
  assert.match(html, /<h1>Rendered by the browser<\/h1>/);
  assert.equal(isSkeleton(html), false);
  const m = metricsJson();
  assert.ok(m.rendered >= 1 && m.misses >= 1 && m.fresh >= 1 && m.renderP50 !== null);
});

test("mount=append wraps the snapshot for apps that append to <body>", async () => {
  const r = await fetch(`${rbase}/refresh`, { method: "POST", headers: { ...auth, "content-type": "application/json" }, body: JSON.stringify({ urls: [base + "/a"], mount: "append" }) });
  assert.deepEqual(await r.json(), { queued: 1 });
  let html = "";
  for (let i = 0; i < 100 && !html; i++) {
    await new Promise((r) => setTimeout(r, 200));
    const s = await fetch(`${rbase}/snapshot?mount=append&url=${encodeURIComponent(base + "/a")}`, { headers: auth });
    if (s.status === 200) html = await s.text();
  }
  assert.match(html, /<div data-glot-prerender="" style="display: contents;"><div id="root"><h1>/);
  assert.match(html, /<\/div><script>/, "scripts stay outside the wrapper");
});

test("images, media, fonts, analytics are never fetched", () => {
  for (const t of ["image", "media", "font", "websocket"]) assert.equal(blockedRequest(t, "cdn.example.com"), true, t);
  for (const h of ["www.google-analytics.com", "www.googletagmanager.com", "static.hotjar.com", "us.i.posthog.com", "connect.facebook.net"]) assert.equal(blockedRequest("script", h), true, h);
  assert.equal(blockedRequest("script", "cdn.example.com"), false);
  assert.equal(blockedRequest("fetch", "api.example.com"), false);
  assert.equal(blockedRequest("script", "notgoogle-analytics.com"), false, "suffix match on a label boundary only");
});

test("one browser, recycled after N renders, stopped when idle", async () => {
  config.allowPrivate = true;
  await stopBrowser(); // reset the render counter left by earlier tests
  const before = metricsJson();
  config.recycleAfter = 2;
  await render(base + "/r1");
  await render(base + "/r2");
  assert.equal(metricsJson().recycles, before.recycles + 1, "recycled after 2 renders");
  assert.equal(metricsJson().browser, "stopped");
  config.recycleAfter = 50;
  config.idleMs = 300;
  const r = await fetch(`${rbase}/refresh`, { method: "POST", headers: { ...auth, "content-type": "application/json" }, body: JSON.stringify({ urls: [base + "/idle"] }) });
  assert.deepEqual(await r.json(), { queued: 1 });
  let up = false;
  for (let i = 0; i < 100 && metricsJson().rendered === before.rendered; i++) { await new Promise((r) => setTimeout(r, 100)); up ||= metricsJson().browser === "up"; }
  for (let i = 0; i < 30 && metricsJson().browser === "up"; i++) await new Promise((r) => setTimeout(r, 100));
  const m = metricsJson();
  assert.equal(m.browser, "stopped", "closed after the idle delay");
  assert.ok(m.idleStops > before.idleStops && m.launches > before.launches);
  config.idleMs = 60000;
});

test("a hung page is killed at the timeout and the next render starts clean", async () => {
  config.timeoutMs = 1500;
  const t0 = performance.now();
  await assert.rejects(render(base + "/hang"));
  assert.ok(performance.now() - t0 < 8000, "bounded by timeout + grace");
  config.timeoutMs = 15000;
  assert.match(await render(base + "/after-hang"), /Rendered by the browser/);
});

test("CLI prerender writes one file per route and a manifest", async () => {
  assert.equal(routeFile("/"), "index.html");
  assert.equal(routeFile("/pricing/"), join("pricing", "index.html"));
  assert.equal(routeFile("/a/../etc"), null);
  assert.equal(routeFile("/a?b=1"), null);
  const out = mkdtempSync(join(tmpdir(), "glot-prerender-"));
  const r = await prerender(base, ["/", "/pricing", "/../x"], out);
  assert.deepEqual(r.snapshots.map((s) => s.path), ["/", "/pricing"]);
  assert.equal(r.failed.length, 1);
  assert.match(readFileSync(join(out, "pricing", "index.html"), "utf8"), /<h1>Rendered by the browser<\/h1>/);
  const manifest = JSON.parse(readFileSync(join(out, "manifest.json"), "utf8"));
  assert.equal(manifest.snapshots[1].file, join("pricing", "index.html"));
  assert.equal(metricsJson().browser, "stopped", "the CLI closes the browser");
});
