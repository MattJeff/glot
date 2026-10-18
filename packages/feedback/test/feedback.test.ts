// E2E (Playwright): static page + fake glot endpoint. Run `pnpm build` first.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, type Browser } from "playwright";

const js = readFileSync(join(import.meta.dirname, "../dist/feedback.global.js"), "utf8");
const posts: { path: string; type: string; body: any }[] = [];
let web: Server, browser: Browser, base = "";

before(async () => {
  web = createServer((req, res) => {
    if (req.method === "POST") {
      let b = "";
      req.on("data", (c) => (b += c));
      req.on("end", () => {
        posts.push({ path: req.url!, type: String(req.headers["content-type"]), body: JSON.parse(b) });
        res.writeHead(202, { "content-type": "application/json" }).end('{"matched":true}');
      });
      return;
    }
    if (req.url === "/feedback.js") return res.writeHead(200, { "content-type": "text/javascript" }).end(js);
    res.writeHead(200, { "content-type": "text/html" }).end(`<!doctype html><html lang="fr"><body>
      <main><h1 id="greet">Salut Ana !</h1><p>Autre texte</p></main>
      <script src="/feedback.js" data-key="pk_test"></script></body></html>`);
  });
  await new Promise<void>((ok) => web.listen(0, "127.0.0.1", ok));
  base = `http://127.0.0.1:${(web.address() as any).port}`;
  browser = await chromium.launch();
});

after(async () => {
  await browser?.close();
  web?.close();
});

test("select text → Improve translation → accessible form → report sent", async () => {
  const page = await browser.newPage();
  await page.goto(base + "/pricing?email=a@b.c#top");
  const pill = page.getByRole("button", { name: "Improve translation" });
  assert.equal(await pill.isVisible(), false);
  await page.evaluate(() => {
    const r = document.createRange();
    r.selectNodeContents(document.getElementById("greet")!);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(r);
    document.dispatchEvent(new MouseEvent("mouseup"));
  });
  await pill.click();
  const dialog = page.getByRole("dialog", { name: "Is this translation wrong?" });
  await dialog.waitFor();
  assert.equal(await dialog.getByLabel("Text").inputValue(), "Salut Ana !");
  await dialog.getByRole("radio", { name: "Awkward" }).check();
  await dialog.getByLabel("Better wording (optional)").fill("Bonjour Ana !");
  await dialog.getByRole("button", { name: "Send" }).click();
  await dialog.getByRole("status").getByText("Thanks!").waitFor();
  await dialog.waitFor({ state: "hidden" });
  assert.equal(posts.length, 1);
  const p = posts[0];
  assert.equal(p.path, "/api/feedback/pk_test");
  assert.match(p.type, /text\/plain/); // simple CORS request, no preflight
  assert.deepEqual(p.body, {
    locale: "fr",
    text: "Salut Ana !",
    kind: "awkward",
    suggestion: "Bonjour Ana !",
    url: base + "/pricing",
    dom: "main>h1#greet",
  });
});

test("runtime badge's glot:improve opens the form; Escape closes it", async () => {
  const page = await browser.newPage();
  await page.goto(base + "/");
  const prevented = await page.evaluate(() => {
    const ev = new CustomEvent("glot:improve", { bubbles: true, cancelable: true, detail: { locale: "fr", key: "home.greet", text: "Salut Ana !" } });
    return !document.body.dispatchEvent(ev);
  });
  assert.equal(prevented, true, "the badge link is not followed");
  const dialog = page.getByRole("dialog", { name: "Is this translation wrong?" });
  await dialog.waitFor();
  // Focus lands inside the modal (native <dialog>), on the first choice.
  assert.equal(await dialog.getByRole("radio", { name: "Wrong" }).evaluate((e) => e === (e.getRootNode() as ShadowRoot).activeElement), true);
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "hidden" });
  await page.evaluate(() => (window as any).glotFeedback.open("Autre texte", "home.other"));
  await dialog.getByRole("button", { name: "Send" }).click();
  await dialog.waitFor({ state: "hidden" });
  assert.equal(posts.at(-1)!.body.key, "home.other");
});
