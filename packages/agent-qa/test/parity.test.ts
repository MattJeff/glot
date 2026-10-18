import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type Anthropic from "@anthropic-ai/sdk";
import { Ctx, attribute, run, score, type Options } from "../src/parity.ts";

// A static bilingual app with one defect per locale:
// fr: "Nouvelle facture" means both "New invoice" and "Create invoice" (ambiguous), the reminder button is cut off,
//     and the reminder toast is missing from the catalog (falls back to English).
// de: the reminder button has no translation (falls back to English): a German-only reader can't find it.
// ar: no dir="rtl".
const cat: Record<string, Record<string, string>> = {
  en: { "inv.title": "Invoices", "inv.new": "New invoice", "inv.create": "Create invoice", "inv.save": "Save", "inv.created": "Invoice for {customer} created", "inv.remind": "Send reminder", "inv.reminded": "Reminder sent" },
  fr: { "inv.title": "Factures", "inv.new": "Nouvelle facture", "inv.create": "Nouvelle facture", "inv.save": "Enregistrer", "inv.created": "Facture pour {customer} créée", "inv.remind": "Envoyer un rappel au client" },
  de: { "inv.title": "Rechnungen", "inv.new": "Neue Rechnung", "inv.create": "Rechnung erstellen", "inv.save": "Speichern", "inv.created": "Rechnung für {customer} erstellt", "inv.reminded": "Erinnerung gesendet" },
  ar: { "inv.title": "الفواتير", "inv.new": "فاتورة جديدة", "inv.create": "إنشاء فاتورة", "inv.save": "حفظ", "inv.created": "تم إنشاء فاتورة {customer}", "inv.remind": "إرسال تذكير", "inv.reminded": "تم إرسال التذكير" },
};

const page = (l: string) => {
  const t = (k: string) => cat[l]?.[k] ?? cat.en[k];
  const js = (k: string) => JSON.stringify(t(k)).replaceAll('"', "&quot;");
  return `<!doctype html><html lang="${l}"><body>
<h1>${t("inv.title")}</h1>
<button id="new" onclick="document.getElementById('f').hidden=false">${t("inv.new")}</button>
<a href="#" onclick="document.getElementById('f').hidden=false">${t("inv.create")}</a>
<form id="f" hidden onsubmit="event.preventDefault();toast(${js("inv.created")}.replace('{customer}', this.customer.value))">
  <label>Customer <input name="customer"></label>
  <select name="currency"><option>USD</option><option>EUR</option></select>
  <button type="submit">${t("inv.save")}</button>
</form>
<button style="width:120px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis" onclick="toast(${js("inv.reminded")})">${t("inv.remind")}</button>
<div aria-live="polite" id="toast"></div>
<script>function toast(m){document.getElementById('toast').innerHTML='<p>'+m+'</p>'}</script>
</body></html>`;
};

const tasks = `
- id: create_invoice
  goal: Create an invoice for Acme in euros
  start: /
  success: { text_visible_key: inv.created }
  script:
    - click: inv.new
    - type: { selector: "input[name=customer]", text: Acme }
    - select: { selector: "select[name=currency]", value: EUR }
    - click: { key: inv.save }
- id: send_reminder
  goal: Remind the customer about their invoice
  success: { text_visible_key: inv.reminded }
  script:
    - click: inv.remind
`;

async function fixture(taskYaml: string) {
  const dir = await mkdtemp(join(tmpdir(), "agent-qa-"));
  for (const [l, c] of Object.entries(cat)) await writeFile(join(dir, `${l}.json`), JSON.stringify(c));
  await writeFile(join(dir, "tasks.yaml"), taskYaml);
  const server = createServer((req, res) => {
    const l = new URL(req.url!, "http://x").searchParams.get("lang") || "en";
    res.setHeader("content-type", "text/html; charset=utf-8");
    res.end(page(l));
  });
  await new Promise<void>((r) => server.listen(0, r));
  const port = (server.address() as { port: number }).port;
  const opts = (locales: string[], provider: "mock" | "anthropic"): Options => ({
    tasks: join(dir, "tasks.yaml"), baseUrl: `http://localhost:${port}`, locales, sourceLocale: "en",
    urlPattern: "{route}?lang={locale}", catalogs: join(dir, "{locale}.json"), provider, model: "claude-sonnet-5-5", maxSteps: 5,
    viewport: "1024x700", out: join(dir, "out"),
  });
  const close = async () => { server.close(); await rm(dir, { recursive: true, force: true }); };
  return { dir, opts, close };
}

test("mock agent: parity matrix, findings tied to keys", async () => {
  const { dir, opts, close } = await fixture(tasks);
  try {
    const report = await run(opts(["fr", "de", "ar"], "mock"));
    const r = (task: string, l: string) => report.runs.find((x) => x.task === task && x.locale === l)!;
    const kinds = (task: string, l: string) => r(task, l).findings.map((f) => `${f.kind}:${f.key ?? ""}`).sort();

    assert.deepEqual(report.options.locales, ["en", "fr", "de", "ar"]);
    assert.ok(r("create_invoice", "en").success && r("send_reminder", "en").success);
    assert.equal(r("create_invoice", "en").steps, 4);
    assert.deepEqual(kinds("create_invoice", "en"), []);

    assert.ok(r("create_invoice", "fr").success);
    assert.deepEqual(kinds("create_invoice", "fr"), ["ambiguous:inv.new"]);
    assert.ok(r("send_reminder", "fr").success);
    assert.deepEqual(kinds("send_reminder", "fr"), ["truncated:inv.remind", "untranslated:inv.reminded"]);

    assert.equal(r("send_reminder", "de").success, false);
    assert.equal(r("send_reminder", "de").reason, "untranslated");
    assert.equal(r("send_reminder", "de").findings[0].suggestion, "glot translate --locale de --key inv.remind");

    assert.ok(r("create_invoice", "ar").success);
    assert.ok(kinds("create_invoice", "ar").includes("rtl:"));

    assert.deepEqual(Object.fromEntries(Object.entries(report.scores).map(([l, s]) => [l, s.parity])), { en: 100, fr: 100, de: 50, ar: 100 });
    const md = await readFile(join(dir, "out", "report.md"), "utf8");
    assert.match(md, /\| send_reminder \| ✅ 1 \| ✅ 1 \| ❌ untranslated \| ✅ 1 \|/);
    assert.ok(r("create_invoice", "fr").trace.every((s) => s.screenshot.endsWith(".png")));
  } finally {
    await close();
  }
});

test("LLM give_up is attributed to the key it meant", () => {
  const ctx = new Ctx({ en: { "inv.remind": "Send reminder", "inv.save": "Save" }, fr: { "inv.remind": "Envoyer un rappel", "inv.save": "Enregistrer" } }, "en");
  const f = attribute(ctx, "fr", { action: "give_up", reason: "not_found", looked_for: "Relancer", looked_for_source: "Send a reminder", detail: "" });
  assert.equal(f.key, "inv.remind");
  assert.match(f.detail, /"Envoyer un rappel" was looked for as "Relancer"/);
  assert.equal(f.suggestion, 'glot set fr inv.remind "Relancer"');
});

test("parity is relative to the source locale", () => {
  const run = (task: string, locale: string, success: boolean) => ({ task, locale, goal: "", success, steps: 2, ms: 10, findings: [], trace: [] });
  const s = score([run("a", "en", true), run("b", "en", false), run("a", "fr", false), run("b", "fr", true), run("a", "de", true), run("b", "de", true)], ["en", "fr", "de"], "en");
  // fr only succeeds where the source fails: that does not count.
  assert.deepEqual([s.en.parity, s.fr.parity, s.de.parity, s.fr.rate, s.de.avg_steps], [100, 0, 100, 50, 2]);
});

test("Claude agent loop (stubbed client): request shape, goal cache, attribution", async () => {
  const { opts, close } = await fixture(`
- id: send_reminder
  goal: Remind the customer about their invoice
  success: { text_visible_key: inv.reminded }
`);
  const requests: any[] = [];
  let translations = 0;
  const label: Record<string, string> = { en: "Send reminder", fr: "Envoyer un rappel" };
  const stub = {
    messages: { create: async (req: any) => { translations++; return { stop_reason: "end_turn", content: [{ type: "text", text: `[goal] ${req.messages[0].content.split("\n\n")[1]}` }] }; } },
    beta: { messages: { create: async (req: any) => {
      requests.push(structuredClone(req));
      const locale = /native "(\w+)"/.exec(req.system)![1];
      const last = req.messages.at(-1).content;
      const tree = (last[0].type === "tool_result" ? last[0].content : last).find((b: any) => b.type === "text").text as string;
      const ref = new RegExp(`\\[(\\d+)\\] button "${label[locale] ?? "-"}`).exec(tree)?.[1];
      const input = ref ? { ref: Number(ref) } : { reason: "not_found", looked_for: "Erinnerung senden", looked_for_source: "Send reminder", detail: "" };
      return { stop_reason: "tool_use", content: [{ type: "tool_use", id: `t${requests.length}`, name: ref ? "click" : "give_up", input }] };
    } } },
  } as unknown as Anthropic;
  try {
    const report = await run(opts(["fr", "de"], "anthropic"), stub);
    const r = (l: string) => report.runs.find((x) => x.locale === l)!;
    assert.ok(r("en").success && r("fr").success);
    assert.equal(r("fr").goal, "[goal] Remind the customer about their invoice");
    assert.equal(r("de").success, false);
    assert.equal(r("de").reason, "not_found");
    const f = r("de").findings.find((x) => x.kind === "not_found")!;
    assert.equal(f.key, "inv.remind");
    assert.equal(f.suggestion, 'glot set de inv.remind "Erinnerung senden"');

    const q = requests[0];
    assert.equal(q.model, "claude-sonnet-5-5");
    assert.equal(q.fallbacks, "default");
    assert.deepEqual(q.betas, ["server-side-fallback-2026-07-01"]);
    assert.ok(q.tools.every((t: any) => t.strict));
    assert.equal(q.messages[0].content[0].type, "image");
    assert.equal(translations, 2);
    await run(opts(["fr", "de"], "anthropic"), stub);
    assert.equal(translations, 2, "goals are translated once and cached");
  } finally {
    await close();
  }
});
