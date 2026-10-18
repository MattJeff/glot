// E2E (Playwright): static page + fake server following the contract. `pnpm build` first.
// SHOTS=<dir> also saves screenshots there.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { chromium, type Browser, type Page } from "playwright";
import { serve } from "./fake.ts";

const SHOTS = process.env.SHOTS;
if (SHOTS) mkdirSync(SHOTS, { recursive: true });
const shot = (p: Page, name: string) => SHOTS && p.screenshot({ path: join(SHOTS, name + ".png"), animations: "disabled" });
const AXE = join(import.meta.dirname, "../../../web/node_modules/.pnpm/axe-core@4.14.0/node_modules/axe-core/axe.min.js");

/** axe-core on the widget's shadow tree (skipped when web/'s axe-core isn't installed). */
async function axe(page: Page) {
  if (!existsSync(AXE)) return;
  await page.addScriptTag({ content: readFileSync(AXE, "utf8") });
  const v = await page.evaluate(async () => (await (window as any).axe.run(document.querySelector("glot-review"), { runOnly: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"] })).violations.map((x: any) => `${x.id}: ${x.nodes.map((n: any) => n.target.join(" ")).join(", ")}`));
  assert.deepEqual(v, []);
}

let srv: Awaited<ReturnType<typeof serve>>, browser: Browser;
before(async () => { srv = await serve(Number(process.env.PORT) || 3502); browser = await chromium.launch(); });
after(async () => { await browser?.close(); srv?.close(); });

async function open(token: string | null, opts: Parameters<Browser["newPage"]>[0] = {}, q = "") {
  const page = await browser.newPage({ locale: "fr-FR", ...opts });
  page.setDefaultTimeout(5000);
  page.on("pageerror", (e) => console.error("pageerror:", e.message));
  const js: string[] = [];
  page.on("request", (r) => r.url().endsWith(".js") && js.push(r.url().split("/").pop()!));
  await page.goto(`${srv.base}/${q ? "?" + q : ""}${token ? (q ? "&" : "?") + "glot_review=" + token : ""}`);
  if (token) await page.getByRole("toolbar", { name: "Revue glot" }).waitFor();
  return { page, js };
}
const pop = (p: Page) => p.getByRole("dialog");
/** In comment mode the widget's overlay sits above the page: click through coordinates, like a user. */
async function pick(page: Page, target: ReturnType<Page["locator"]>) {
  const b = (await target.boundingBox())!;
  await page.mouse.click(b.x + Math.min(20, b.width / 2), b.y + b.height / 2);
}

test("normal visitor: loader only, nothing rendered, no API call", async () => {
  const before = srv.log.length;
  const { page, js } = await open(null);
  await page.waitForLoadState("networkidle");
  assert.deepEqual(js, ["review.global.js"]);
  assert.equal(await page.locator("glot-review").count(), 0);
  assert.equal(srv.log.length, before);
  await page.close();
});

test("review link: token leaves the URL, keyboard annotation with key, pin + capture", async () => {
  const { page, js } = await open("glr_edit");
  assert.ok(js.includes("review.app.global.js"));
  assert.ok(!page.url().includes("glot_review"));
  await shot(page, "01-toolbar");
  // Keyboard only: focus Comment, Enter, Tab to the title, Enter.
  await page.getByRole("button", { name: "Commenter" }).focus();
  await page.keyboard.press("Enter");
  const live = page.locator("glot-review").locator(".ui > [role=status]");
  for (let i = 0; i < 5 && !/home\.title/.test((await live.textContent()) || ""); i++) { await page.keyboard.press("Tab"); await page.waitForTimeout(80); }
  assert.match((await live.textContent())!, /^3\/\d+: Traduisez votre app en 5 minutes \(home\.title\)$/);
  await shot(page, "02-keyboard-pick");
  await page.keyboard.press("Enter");
  const d = pop(page);
  await d.getByRole("heading", { name: "Nouvelle annotation" }).waitFor();
  assert.equal(await d.locator("code").first().textContent(), "home.title");
  await d.getByLabel("Commentaire").fill("Trop long pour le **hero**.");
  await shot(page, "03-new-popover");
  await axe(page);
  await d.getByRole("button", { name: "Envoyer" }).click();
  await d.waitFor({ state: "hidden" });
  const pin = page.getByRole("button", { name: /^#1 Ouverte / });
  await pin.waitFor();
  const create = srv.log.find((l) => l.method === "POST" && l.path === "/annotations")!;
  assert.equal(create.body.anchor.key, "home.title");
  assert.equal(create.body.anchor.key_source, "attribute");
  assert.equal(create.body.anchor.locale, "fr");
  assert.equal(create.body.anchor.text, "Traduisez votre app en 5 minutes");
  assert.equal(create.body.author_name, "Ana (lien)");
  await page.waitForFunction(() => true);
  for (let i = 0; i < 20 && !srv.log.some((l) => l.path.includes("/attachments")); i++) await page.waitForTimeout(100);
  assert.ok(srv.log.some((l) => l.method === "POST" && l.path.includes("/attachments")), "capture uploaded");
  await shot(page, "04-pin");
  // Pin follows scroll.
  const y0 = (await pin.boundingBox())!.y;
  await page.evaluate(() => scrollTo(0, 120));
  await page.waitForFunction((y) => { const p = document.querySelector("glot-review")!.shadowRoot!.querySelector(".pin")!; return Math.abs(p.getBoundingClientRect().y - (y - 120)) < 3; }, y0);
  await page.close();
});

test("thread: reply, react, edit, status, filter, delete", async () => {
  const { page } = await open("glr_edit");
  await page.getByRole("button", { name: /^#1/ }).click();
  const d = pop(page);
  await d.getByRole("tab", { name: "Discussion" }).waitFor();
  await d.getByLabel("Commentaire").fill("D'accord, à raccourcir");
  await d.getByRole("button", { name: "Répondre" }).click();
  await d.getByText("D'accord, à raccourcir").waitFor();
  await d.getByRole("button", { name: "+1 (0)" }).first().click();
  await d.getByRole("button", { name: "+1 (1)" }).first().waitFor();
  await shot(page, "05-thread");
  await axe(page);
  // Edit own comment.
  await d.locator("li.cm").getByRole("button", { name: "Modifier" }).click();
  await d.getByLabel("Modifier").fill("À raccourcir, oui");
  await d.getByRole("button", { name: "Enregistrer" }).click();
  await d.getByText("À raccourcir, oui").waitFor();
  await d.getByRole("combobox", { name: "Statut" }).selectOption("resolved");
  await d.getByRole("combobox", { name: "Statut" }).and(page.locator("select")).first().waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: /^#1/ }).waitFor({ state: "detached" });
  await page.getByRole("toolbar").getByRole("combobox", { name: "Statut" }).selectOption({ label: "Résolues" });
  await page.getByRole("button", { name: /^#1 Résolue / }).waitFor();
  await page.close();
});

test("✨ reword: creates the annotation, suggestion previewed in place, applied", async () => {
  const { page } = await open("glr_edit");
  await page.getByRole("button", { name: "Commenter" }).click();
  await pick(page, page.getByTestId("lead"));
  const d = pop(page);
  await d.getByText("Proposer un nouveau texte").click();
  await d.getByRole("button", { name: "Plus court" }).click();
  await d.getByRole("group", { name: "Propositions" }).waitFor();
  await page.waitForFunction(() => document.querySelector("[data-testid=lead]")!.textContent === "Votre produit parle déjà toutes");
  assert.ok(srv.log.some((l) => l.path.endsWith("/rephrase") && l.body.preset === "shorter" && l.body.count === 3));
  await shot(page, "06-rephrase-proposals");
  await d.getByRole("radio", { name: /^Votre produit parle Encore plus court/ }).check();
  await page.waitForFunction(() => document.querySelector("[data-testid=lead]")!.textContent === "Votre produit parle");
  await d.getByRole("button", { name: "Appliquer dans toutes les langues" }).click();
  await d.waitFor({ state: "hidden" });
  // Preview restored, original markup (dev marker) intact.
  assert.match(await page.getByTestId("lead").textContent() as string, /toutes les langues/);
  const a = srv.anns.find((x) => x.key === "home.lead")!;
  assert.equal(a.status, "resolved");
  assert.equal(srv.texts["home.lead"].values.de.value, "[de] Votre produit parle");
  assert.equal(a.anchor.key_source, "marker");
  await page.close();
});

test("manual edit: placeholders protected, live validator, save, revert, delete", async () => {
  const { page } = await open("glr_edit");
  await page.getByRole("button", { name: "Commenter" }).click();
  await pick(page, page.getByText("Vous avez 3 messages"));
  const d = pop(page);
  await d.getByRole("tab", { name: "Texte" }).click();
  const f = d.getByLabel("Texte (fr)");
  await f.waitFor();
  assert.equal(await f.inputValue(), "Vous avez {count} messages");
  await f.fill("Vous avez des messages");
  await d.getByText("Variable manquante ou modifiée : {count}").waitFor();
  assert.equal(await f.getAttribute("aria-invalid"), "true");
  await shot(page, "07-edit-invalid");
  await d.getByRole("button", { name: "Enregistrer" }).click();
  assert.ok(!srv.log.some((l) => l.method === "PUT" && !l.body.dry_run), "blocked client-side");
  await f.fill("{count} nouveaux messages");
  await d.getByText("Validateur : aucun problème").waitFor();
  await d.getByLabel("Aperçu dans la page").check();
  await page.getByText("{count} nouveaux messages").first().waitFor();
  await d.getByRole("button", { name: "Enregistrer" }).click();
  await page.locator("glot-review .ui > [role=status]").filter({ hasText: "Enregistré" }).waitFor({ state: "attached" });
  assert.equal(srv.texts["home.inbox"].values.fr.value, "{count} nouveaux messages");
  await d.getByRole("button", { name: "Version précédente" }).click();
  await page.locator("glot-review .ui > [role=status]").filter({ hasText: "Version précédente restaurée" }).waitFor({ state: "attached" });
  assert.equal(srv.texts["home.inbox"].values.fr.value, "Vous avez {count} messages");
  await d.getByRole("button", { name: "Supprimer la traduction" }).click();
  await d.getByRole("button", { name: "Confirmer la suppression" }).click();
  await page.locator("glot-review .ui > [role=status]").filter({ hasText: "Traduction supprimée" }).waitFor({ state: "attached" });
  assert.equal(srv.texts["home.inbox"].values.fr, undefined);
  await page.close();
});

test("all languages: atomic multi-locale save, server errors shown", async () => {
  const { page } = await open("glr_member");
  await page.getByRole("button", { name: "Commenter" }).click();
  await pick(page, page.getByRole("heading", { level: 1 }));
  const d = pop(page);
  await d.getByRole("tab", { name: "Discussion" }).focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  assert.equal(await d.getByRole("tab", { name: "Toutes les langues" }).getAttribute("aria-selected"), "true");
  await d.getByLabel("de").fill("Übersetze deine App in 5 Minuten");
  await d.getByLabel("ar (manquante)").fill("ترجم تطبيقك في 5 دقائق");
  await shot(page, "08-all-languages");
  await axe(page);
  await d.getByRole("button", { name: "Enregistrer" }).click();
  await page.locator("glot-review .ui > [role=status]").filter({ hasText: "Enregistré" }).waitFor({ state: "attached" });
  const put = srv.log.findLast((l) => l.method === "PUT" && !l.body.dry_run)!;
  assert.deepEqual(Object.keys(put.body.values).sort(), ["ar", "de"]);
  assert.equal(put.body.author_name, undefined, "members send no author_name");
  await page.close();
});

test("hard-coded text: create a key", async () => {
  const { page } = await open("glr_edit");
  await page.getByRole("button", { name: "Commenter" }).click();
  await pick(page, page.locator("#legal"));
  const d = pop(page);
  await d.getByText("clé non détectée").waitFor();
  await d.getByRole("tab", { name: "Texte" }).click();
  await d.getByLabel("Clé (optionnelle, générée si vide)").fill("legal.notice");
  await d.getByRole("button", { name: "Créer une clé à partir de ce texte" }).click();
  await d.locator("code", { hasText: "legal.notice" }).waitFor();
  assert.ok(srv.texts["legal.notice"]);
  await page.close();
});

test("realtime: another reviewer's annotation appears and is announced; orphans listed", async () => {
  const { page } = await open("glr_edit");
  const n = await page.locator("glot-review").evaluate((h) => h.shadowRoot!.querySelectorAll(".pin").length);
  const other = await open("glr_member");
  await other.page.getByRole("button", { name: "Commenter" }).click();
  await pick(other.page, other.page.locator("#cta"));
  await pop(other.page).getByLabel("Commentaire").fill("Bouton trop long en allemand");
  await pop(other.page).getByRole("button", { name: "Envoyer" }).click();
  await page.locator("glot-review .ui > [role=status]").filter({ hasText: "Nouvelle annotation de Mathis" }).waitFor({ state: "attached" });
  await page.waitForFunction((n) => document.querySelector("glot-review")!.shadowRoot!.querySelectorAll(".pin").length === n + 1, n);
  // Orphan: text gone from the page.
  srv.anns.push({ ...srv.anns.at(-1), id: "00000000-0000-4000-8000-000000000001", key: null, anchor: { ...srv.anns.at(-1).anchor, key: null, selector: "#nope", text: "Texte disparu" }, created_at: new Date().toISOString() });
  await page.reload();
  await page.getByRole("button", { name: /^Annotations/ }).click();
  await page.getByText("orphelin : texte introuvable sur cette page").waitFor();
  await shot(page, "09-list-orphan");
  await other.page.close(); await page.close();
});

test("read-only link: no write controls; expired link explains", async () => {
  const { page } = await open("glr_read");
  assert.equal(await page.getByRole("button", { name: "Commenter" }).isVisible(), false);
  await page.close();
  const p2 = await browser.newPage({ locale: "fr-FR" });
  await p2.goto(`${srv.base}/?glot_review=glr_nope`);
  await p2.getByRole("alert").filter({ hasText: "Ce lien de revue a expiré" }).waitFor();
  await p2.close();
});

test("comment link: name asked once, proposal submitted for approval", async () => {
  const { page } = await open("glr_comment");
  await page.getByRole("button", { name: "Commenter" }).click();
  await pick(page, page.getByRole("heading", { level: 1 }));
  const d = pop(page);
  await d.getByRole("tab", { name: "Texte" }).click();
  await d.getByLabel("Votre nom").fill("Zoé");
  await d.getByLabel("Texte (fr)").fill("Votre app traduite en 5 minutes");
  await d.getByRole("button", { name: "Soumettre pour validation" }).click();
  await d.getByText("Suggestion").waitFor();
  const a = srv.anns.findLast((x) => x.author.name === "Zoé")!;
  assert.equal(a.suggestion, "Votre app traduite en 5 minutes");
  await page.close();
});

test("mobile + RTL + dark: long press annotates; axe finds no violation", async () => {
  const { page } = await open("glr_edit", { viewport: { width: 375, height: 740 }, hasTouch: true, isMobile: true, colorScheme: "dark", reducedMotion: "reduce" }, "lang=ar&dir=rtl&wdir=rtl");
  await shot(page, "10-mobile-rtl");
  const h1 = await page.getByRole("heading", { level: 1 }).boundingBox();
  const cdp = await page.context().newCDPSession(page);
  const pt = [{ x: h1!.x + 20, y: h1!.y + 10 }];
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: pt });
  await page.waitForTimeout(700);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await pop(page).getByRole("heading", { name: "Nouvelle annotation" }).waitFor();
  const box = (await pop(page).boundingBox())!;
  assert.ok(box.x >= 0 && box.x + box.width <= 375, "popover fits the phone");
  await shot(page, "11-mobile-popover");
  await axe(page);
  await page.close();
});

test("iframe protocol (§5): pick → picked anchor, preview, origin check", async () => {
  const port = new URL(srv.base).port;
  const app = `http://localhost:${port}`; // the "glot web app" origin; the site is 127.0.0.1
  const site = `${srv.base}/?appo=${encodeURIComponent(app)}&glot_review=glr_edit`;
  const page = await browser.newPage({ locale: "fr-FR" });
  page.setDefaultTimeout(5000);
  page.on("pageerror", (e) => console.error("pageerror:", e.message));
  await page.goto(`${app}/parent?src=${encodeURIComponent(site)}`);
  const msgs = () => page.evaluate(() => (window as any).msgs as any[]);
  await page.waitForFunction(() => (window as any).msgs.some((m: any) => m.type === "glot:review:ready"));
  const ready = (await msgs()).find((m) => m.type === "glot:review:ready");
  assert.equal(ready.origin, srv.base);
  assert.deepEqual([ready.route, ready.locale], ["/", "fr"]);

  // pick: the next click returns an Anchor to the page instead of opening the widget's composer.
  await page.evaluate(() => (window as any).send({ type: "glot:review:mode", annotate: false, pick: true }));
  const frame = page.frameLocator("#f");
  await frame.getByRole("button", { name: "Commenter", pressed: true }).waitFor();
  await pick(page, frame.getByRole("heading", { level: 1 }) as any);
  await page.waitForFunction(() => (window as any).msgs.some((m: any) => m.type === "glot:review:picked"));
  const { anchor } = (await msgs()).find((m) => m.type === "glot:review:picked");
  assert.equal(anchor.key, "home.title");
  assert.equal(anchor.key_source, "attribute");
  assert.equal(anchor.text, "Traduisez votre app en 5 minutes");
  assert.ok(anchor.selector && anchor.rect && anchor.viewport);
  assert.equal(await frame.getByRole("dialog").count(), 0, "no widget composer in pick mode");

  // Escape in pick mode → glot:pick-cancel.
  await page.evaluate(() => (window as any).send({ type: "glot:review:mode", annotate: false, pick: true }));
  await frame.getByRole("button", { name: "Commenter", pressed: true }).waitFor();
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => (window as any).msgs.some((m: any) => m.type === "glot:pick-cancel"));

  // preview: replace the anchored text, then restore it.
  const h1 = frame.getByRole("heading", { level: 1 });
  await page.evaluate((a) => (window as any).send({ type: "glot:review:preview", annotation_id: null, anchor: { key: a.key, selector: a.selector, text: a.text }, value: "Votre app, traduite" }), anchor);
  await h1.filter({ hasText: /^Votre app, traduite$/ }).waitFor();
  await shot(page, "12-iframe-preview");
  await page.evaluate(() => (window as any).send({ type: "glot:review:preview", annotation_id: null, anchor: null, value: null }));
  await h1.filter({ hasText: /^Traduisez votre app en 5 minutes$/ }).waitFor();
  await page.close();

  // Same messages from an origin that is not data-app-origin: ignored.
  const evil = await browser.newPage({ locale: "fr-FR" });
  evil.setDefaultTimeout(5000);
  await evil.goto(`${srv.base}/parent?src=${encodeURIComponent(site)}`);
  await evil.frameLocator("#f").getByRole("toolbar").waitFor();
  await evil.evaluate(() => (window as any).send({ type: "glot:review:mode", annotate: false, pick: true }));
  await evil.evaluate(() => (window as any).send({ type: "glot:review:preview", annotation_id: null, anchor: { selector: "h1", text: "Traduisez votre app en 5 minutes" }, value: "pwned" }));
  await evil.waitForTimeout(300);
  assert.equal(await evil.frameLocator("#f").getByRole("heading", { level: 1 }).textContent(), "Traduisez votre app en 5 minutes");
  assert.equal(await evil.frameLocator("#f").getByRole("button", { name: "Commenter" }).getAttribute("aria-pressed"), "false");
  assert.ok(!(await evil.evaluate(() => (window as any).msgs as any[])).length, "nothing posted to a foreign parent");
  await evil.close();
});
