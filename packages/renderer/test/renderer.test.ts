import { test, after } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isSkeleton } from "../src/skeleton.ts";

process.env.RENDERER_DIR = mkdtempSync(join(tmpdir(), "glot-renderer-"));
process.env.RENDERER_TOKEN = "t0k";
const { config, handle, publicHost, metricsJson } = await import("../src/server.ts");

// a CSR app: empty shell, content rendered by JS after a fetch
const SHELL = `<!DOCTYPE html><html><head><title>App</title></head><body><div id="root"></div><script>
fetch("/data").then(r => r.json()).then(d => { document.getElementById("root").innerHTML = "<h1>" + d.title + "</h1><p>" + "Long enough paragraph. ".repeat(12) + "</p>"; });
</script></body></html>`;
const origin: Server = createServer((req, res) => {
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
after(() => { origin.close(); api.close(); setTimeout(() => process.exit(0), 50); });

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
