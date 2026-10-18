// Against a real glot-server (review_live, contract v1.3). Skipped unless GLOT_REAL=http://127.0.0.1:8841
// (server started with DATABASE_URL pointing at a migrated database). Creates its own user + project.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { chromium, type Browser, type Page } from "playwright";
import { page as html } from "./fake.ts";

const API = process.env.GLOT_REAL;
const PORT = Number(process.env.PORT) || 3505;
const SITE = `http://127.0.0.1:${PORT}`;
const slug = "rv2-" + Date.now().toString(36);
let cookie = "", web: Server, browser: Browser, edit = "", member = "";

async function m(method: string, path: string, body?: unknown) {
  const r = await fetch(API + path, { method, headers: { "content-type": "application/json", cookie }, body: body === undefined ? undefined : JSON.stringify(body) });
  const j = await r.json().catch(() => null);
  if (!r.ok) throw new Error(`${method} ${path} → ${r.status} ${JSON.stringify(j)}`);
  return j;
}

before(async () => {
  if (!API) return;
  const r = await fetch(API + "/api/auth/signup", { method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: `${slug}@example.com`, password: "correct horse battery", name: "Rv Two" }) });
  assert.ok(r.ok, await r.clone().text());
  cookie = r.headers.get("set-cookie")!.split(";")[0];
  await m("POST", "/api/projects", { name: "RV2", slug, source_locale: "en", target_locales: ["fr", "de"] });
  for (const [k, src, fr] of [["home.title", "Translate your app in 5 minutes", "Traduisez votre app en 5 minutes"],
    ["home.lead", "Your product already speaks your customers' languages.", "Votre produit parle déjà toutes les langues de vos clients."],
    ["home.inbox", "You have {count} messages", "Vous avez {count} messages"]]) {
    await m("POST", `/api/projects/${slug}/keys`, { key: k, source: src });
    await m("PUT", `/api/projects/${slug}/keys/${k}/translations/fr`, { value: fr });
  }
  await m("PUT", `/api/projects/${slug}/review/settings`, { origins: [SITE] });
  edit = (await m("POST", `/api/projects/${slug}/review/links`, { label: "Agence", access: "edit" })).token;
  member = (await m("POST", `/api/projects/${slug}/review/session`)).token;
  const js = (f: string) => readFileSync(join(import.meta.dirname, "../dist", f));
  web = createServer((req, res) => {
    const u = new URL(req.url!, SITE);
    if (u.pathname.endsWith(".js")) return res.writeHead(200, { "content-type": "text/javascript" }).end(js(u.pathname.slice(1)));
    res.writeHead(200, { "content-type": "text/html" }).end(html("fr", "ltr", "", "", `data-project="${slug}" data-endpoint="${API}"`));
  });
  await new Promise<void>((ok) => web.listen(PORT, "127.0.0.1", ok));
  browser = await chromium.launch();
});
after(async () => { await browser?.close(); web?.close(); });

async function open(token: string) {
  const p = await browser.newPage({ locale: "fr-FR" });
  p.setDefaultTimeout(8000);
  p.on("pageerror", (e) => console.error("pageerror:", e.message));
  p.on("console", (c) => c.type() === "error" && console.error("console:", c.text()));
  await p.goto(`${SITE}/?glot_review=${token}`);
  await p.getByRole("toolbar", { name: "Revue glot" }).waitFor();
  await p.getByRole("button", { name: "Commenter" }).waitFor();
  return p;
}
async function pick(page: Page, sel: string) {
  await page.getByRole("button", { name: "Commenter" }).click();
  const b = (await page.locator(sel).first().boundingBox())!;
  await page.mouse.click(b.x + 20, b.y + b.height / 2);
  return page.getByRole("dialog");
}
const live = (p: Page, t: string | RegExp) => p.locator("glot-review .ui > [role=status]").filter({ hasText: t }).waitFor({ state: "attached" });

test("real server: annotate, capture, thread, realtime, status", { skip: !API }, async () => {
  const p = await open(edit), other = await open(member);
  const d = await pick(p, "h1");
  await d.getByRole("button", { name: "Envoyer" }).click(); // no name yet: the form must not submit
  await d.getByLabel("Votre nom").fill("Agence Zoé");
  await d.getByLabel("Commentaire").fill("Trop **long**");
  await d.getByRole("button", { name: "Envoyer" }).click();
  await d.waitFor({ state: "hidden" });
  await p.getByRole("button", { name: /^#1 Ouverte/ }).waitFor();
  const list = await m("GET", `/api/projects/${slug}/review/annotations?status=open`);
  const a = list.items[0];
  assert.equal(a.key, "home.title");
  assert.equal(a.author.name, "Agence Zoé");
  for (let i = 0; i < 30 && !(await m("GET", `/api/projects/${slug}/review/annotations/${a.id}`)).attachments.length; i++) await p.waitForTimeout(100);
  assert.equal((await m("GET", `/api/projects/${slug}/review/annotations/${a.id}`)).attachments.length, 1, "capture stored");
  // Realtime: the member sees it, replies; the link reviewer sees the reply.
  await other.getByRole("button", { name: /^#1 Ouverte/ }).waitFor();
  await other.getByRole("button", { name: /^#1/ }).click();
  const od = other.getByRole("dialog");
  await od.locator("strong", { hasText: "long" }).waitFor(); // body_html rendered by the server
  await od.locator("img.shot").waitFor(); // capture served with ?token=
  await other.waitForFunction(() => (document.querySelector("glot-review")!.shadowRoot!.querySelector("img.shot") as HTMLImageElement)?.naturalWidth > 0);
  await od.getByLabel("Commentaire").fill("Ok, je raccourcis");
  await od.getByRole("button", { name: "Répondre" }).click();
  await od.getByText("Ok, je raccourcis").waitFor();
  await od.locator("li.cm").getByRole("button", { name: "+1 (0)" }).click();
  await od.locator("li.cm").getByRole("button", { name: "+1 (1)" }).waitFor();
  await p.getByRole("button", { name: /^#1/ }).click();
  await d.getByText("Ok, je raccourcis").waitFor();
  await d.getByRole("combobox", { name: "Statut" }).selectOption("resolved");
  await p.getByRole("button", { name: /^#1/ }).waitFor({ state: "detached" });
  // Delete own annotation (link: ?author_name=).
  await p.getByRole("toolbar").getByRole("combobox", { name: "Statut" }).selectOption({ label: "Toutes" });
  await p.getByRole("button", { name: /^#1/ }).click();
  await d.getByRole("button", { name: "Supprimer" }).first().click();
  await p.getByRole("button", { name: /^#1/ }).waitFor({ state: "detached" });
  await p.close(); await other.close();
});

test("real server: manual edit, validator, revert, delete, all languages, create key", { skip: !API }, async () => {
  const p = await open(edit);
  let d = await pick(p, "text=Vous avez 3 messages");
  await d.getByRole("tab", { name: "Texte" }).click();
  await d.getByLabel("Votre nom").fill("Zoé");
  const f = d.getByLabel("Texte (fr)");
  await f.fill("Vous avez des messages");
  await d.getByText("Variable manquante ou modifiée : {count}").waitFor();
  await f.fill("{count} nouveaux messages");
  await d.getByText("Validateur : aucun problème").waitFor();
  await d.getByRole("button", { name: "Enregistrer" }).click();
  await live(p, "Enregistré");
  assert.equal((await m("GET", `/api/projects/${slug}/review/text/home.inbox`)).values.fr.value, "{count} nouveaux messages");
  await d.getByRole("button", { name: "Version précédente" }).click();
  await live(p, "Version précédente restaurée");
  assert.equal((await m("GET", `/api/projects/${slug}/review/text/home.inbox`)).values.fr.value, "Vous avez {count} messages");
  await d.getByRole("button", { name: "Supprimer la traduction" }).click();
  await d.getByRole("button", { name: "Confirmer la suppression" }).click();
  await live(p, "Traduction supprimée");
  assert.equal((await m("GET", `/api/projects/${slug}/review/text/home.inbox`)).values.fr.status, "missing");
  await p.keyboard.press("Escape");

  d = await pick(p, "h1");
  await d.getByRole("tab", { name: "Toutes les langues" }).click();
  await d.getByLabel("de (manquante)").fill("Übersetze deine App in 5 Minuten");
  await d.getByRole("button", { name: "Enregistrer" }).click();
  await live(p, "Enregistré");
  assert.equal((await m("GET", `/api/projects/${slug}/review/text/home.title`)).values.de.value, "Übersetze deine App in 5 Minuten");
  await p.keyboard.press("Escape");

  d = await pick(p, "#legal");
  await d.getByRole("tab", { name: "Texte" }).click();
  await d.getByRole("button", { name: "Créer une clé à partir de ce texte" }).click();
  await d.locator("code", { hasText: /^[a-z]/ }).first().waitFor();
  const key = await d.locator("code").first().textContent();
  assert.equal((await m("GET", `/api/projects/${slug}/review/text/${encodeURIComponent(key!)}`)).source, "Texte en dur sans clé.");
  await p.close();
});

test("real server: ✨ reword reaches RV4 (proposals, or a clear error without an LLM key)", { skip: !API }, async () => {
  const p = await open(edit);
  const d = await pick(p, "[data-testid=lead]");
  await d.getByText("Proposer un nouveau texte").click();
  await d.getByRole("button", { name: "Plus court" }).click();
  await d.getByText("Indiquez d'abord votre nom").waitFor(); // guarded before any write
  await d.getByLabel("Votre nom").fill("Zoé");
  await d.getByRole("button", { name: "Plus court" }).click();
  await Promise.race([d.getByRole("group", { name: "Propositions" }).waitFor({ timeout: 60000 }), d.locator("[role=alert]").filter({ hasText: /\S/ }).waitFor({ timeout: 60000 })]);
  console.log("rephrase →", (await d.getByRole("group", { name: "Propositions" }).count()) ? "proposals" : await d.locator("[role=alert]").filter({ hasText: /\S/ }).first().textContent());
  await p.close();
});
