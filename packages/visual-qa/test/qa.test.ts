import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildUrl, run } from "../src/qa.ts";

const page = (lang: string, cta: string, dir = "ltr") => `<!doctype html>
<html lang="${lang}" dir="${dir}"><body style="font-family: sans-serif">
  <h1>Welcome back</h1>
  <button data-glot-key="auth.login.cta" style="width:120px;white-space:nowrap;overflow:hidden">${cta}</button>
</body></html>`;

const pages: Record<string, string> = {
  "/login": page("en", "Sign in"),
  "/de/login": page("de", "Jetzt kostenlos anmelden und loslegen"),
  "/ar/login": page("ar", "تسجيل الدخول"), // dir missing on purpose
};

test("buildUrl: source locale has no prefix", () => {
  const o = { baseUrl: "http://x/", urlPattern: "/{locale}{route}", sourceLocale: "en" };
  assert.equal(buildUrl(o, "en", "/"), "http://x/");
  assert.equal(buildUrl(o, "fr", "/"), "http://x/fr");
  assert.equal(buildUrl(o, "fr", "/login"), "http://x/fr/login");
});

test("detects overflowing German button, untranslated text and missing rtl", async () => {
  const server = createServer((req, res) => {
    const body = pages[req.url!];
    res.writeHead(body ? 200 : 404, { "content-type": "text/html; charset=utf-8" }).end(body ?? "");
  });
  await new Promise<void>((r) => server.listen(0, r));
  const port = (server.address() as { port: number }).port;
  const out = await mkdtemp(join(tmpdir(), "glot-vqa-"));
  try {
    const report = await run({
      baseUrl: `http://localhost:${port}`,
      routes: ["/login"],
      locales: ["de", "ar"],
      sourceLocale: "en",
      urlPattern: "/{locale}{route}",
      viewports: ["1280x800"],
      out,
      suggest: false,
    });
    const de = report.issues.filter((i) => i.locale === "de");
    const overflow = de.find((i) => i.kind === "overflow");
    assert.ok(overflow, JSON.stringify(report.issues, null, 2));
    assert.equal(overflow.key, "auth.login.cta");
    assert.ok(overflow.crop && (await stat(join(out, overflow.crop))).size > 0);
    assert.ok(de.some((i) => i.kind === "untranslated" && i.text === "Welcome back"));
    assert.ok(report.issues.some((i) => i.locale === "ar" && i.kind === "rtl"));
    assert.ok(!report.issues.some((i) => i.locale === "ar" && i.kind === "overflow"));
    assert.match(await readFile(join(out, "report.md"), "utf8"), /\| `de` \| 1 \|/);
  } finally {
    server.close();
  }
});

const serve = async (pages: Record<string, string>) => {
  const server = createServer((req, res) => {
    const body = pages[req.url!];
    res.writeHead(body ? 200 : 404, { "content-type": "text/html; charset=utf-8" }).end(body ?? "");
  });
  await new Promise<void>((r) => server.listen(0, r));
  return { server, baseUrl: `http://localhost:${(server.address() as { port: number }).port}` };
};
const opts = { routes: ["/"], sourceLocale: "en", urlPattern: "/{locale}{route}", viewports: ["800x600"], suggest: false };

test("source locale listed among locales is never untranslated against itself", async () => {
  const html = `<!doctype html><html lang="en"><body><h1>Welcome back</h1></body></html>`;
  const { server, baseUrl } = await serve({ "/": html, "/de": html });
  try {
    const r = await run({ ...opts, baseUrl, locales: ["en", "de"], out: await mkdtemp(join(tmpdir(), "glot-vqa-")) });
    assert.ok(!r.issues.some((i) => i.locale === "en" && i.kind === "untranslated"), JSON.stringify(r.issues));
    assert.ok(r.issues.some((i) => i.locale === "de" && i.kind === "untranslated"));
  } finally {
    server.close();
  }
});

test("ellipsis / line-clamp truncation is clamp, plain clipping stays overflow", async () => {
  const long = "Jetzt kostenlos anmelden und loslegen";
  const html = `<!doctype html><html lang="de"><body style="font-family: sans-serif">
    <p id="ell" style="width:80px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${long}</p>
    <div style="width:80px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap"><span id="anc" style="display:inline-block;width:60px">${long}</span></div>
    <p id="lc" style="width:80px;display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden">${long} ${long}</p>
    <p id="cut" style="width:80px;overflow:hidden;white-space:nowrap">${long}</p>
    <p id="ellv" style="width:400px;height:5px;overflow:hidden;text-overflow:ellipsis">${long}</p>
  </body></html>`;
  const { server, baseUrl } = await serve({ "/": html });
  try {
    // Same page as source and target (pattern without {locale}): only the visual checks matter here.
    const r = await run({ ...opts, baseUrl, sourceLocale: "x", locales: ["de"], urlPattern: "{route}", out: await mkdtemp(join(tmpdir(), "glot-vqa-")) });
    const k = Object.fromEntries(r.issues.filter((i) => i.kind !== "untranslated").map((i) => [i.selector, i.kind]));
    assert.deepEqual(k, { "#ell": "clamp", "#anc": "clamp", "#lc": "clamp", "#cut": "overflow", "#ellv": "overflow" });
  } finally {
    server.close();
  }
});

test("scroll-reveal content is revealed (scrolled, reduced motion) before detection", async () => {
  // Text appears only once its section has been on screen, worded by prefers-reduced-motion.
  const html = (lang: string) => `<!doctype html><html lang="${lang}"><body>
    <h1>${lang === "en" ? "Hello there" : "Hallo zusammen"}</h1>
    <section id="reveal" style="margin-top:3000px;height:50px"></section>
    <script>
      new IntersectionObserver((es) => { for (const e of es) if (e.isIntersecting && !e.target.textContent)
        e.target.textContent = matchMedia("(prefers-reduced-motion: reduce)").matches ? "Revealed calmly" : "Revealed with motion";
      }).observe(document.getElementById("reveal"));
    </script></body></html>`;
  const { server, baseUrl } = await serve({ "/": html("en"), "/de": html("de") });
  try {
    const r = await run({ ...opts, baseUrl, locales: ["de"], out: await mkdtemp(join(tmpdir(), "glot-vqa-")) });
    assert.deepEqual(r.issues.filter((i) => i.kind === "untranslated").map((i) => i.text), ["Revealed calmly"]);
  } finally {
    server.close();
  }
});
