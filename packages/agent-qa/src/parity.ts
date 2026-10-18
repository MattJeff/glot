// Task parity: run the same tasks in every locale, compare success with the source locale, blame labels.
import { chromium, type Page } from "playwright";
import Anthropic from "@anthropic-ai/sdk";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { parse } from "yaml";
import { buildUrl } from "../../visual-qa/src/qa.ts";
import { buildIndex, flatten, match, norm, type Index } from "../../visual-qa/src/keys.ts";
import { act, ClaudeAgent, find, MODEL, observe, translateGoal, type Action, type Decision, type View } from "./agent.ts";

export interface Target { key?: string; selector?: string }
export type Step =
  | { click: Target | string }
  | { type: (Target & { text: string }) }
  /** `option_key`: the option whose label is that key's translation. */
  | { select: (Target & { value?: string; option_key?: string }) }
  | { scroll: "up" | "down" };

export interface Task {
  id: string;
  /** In the source language; translated once per locale for the LLM agent. */
  goal: string;
  start?: string;
  max_steps?: number;
  /** All given conditions must hold. Keys make the check language-independent. */
  success: { url_matches?: string; text_visible_key?: string; selector?: string };
  /** `--provider mock`: deterministic steps. A `key` target is found by its translated label in each locale. */
  script?: Step[];
}

export type Kind = "not_found" | "truncated" | "ambiguous" | "untranslated" | "rtl" | "max_steps" | "check_failed" | "error";

export interface Finding {
  kind: Kind;
  key?: string;
  text?: string;
  detail: string;
  suggestion?: string;
}

export interface Run {
  task: string;
  locale: string;
  goal: string;
  success: boolean;
  steps: number;
  ms: number;
  reason?: Kind;
  findings: Finding[];
  trace: { step: number; action: string; target?: string; key?: string; screenshot: string; error?: string }[];
}

export interface Options {
  tasks: string;
  baseUrl: string;
  locales: string[];
  sourceLocale: string;
  urlPattern: string;
  catalogs?: string;
  provider: "mock" | "anthropic";
  model: string;
  maxSteps: number;
  viewport: string;
  out: string;
  /** Cookie set to the locale before each run (e.g. NEXT_LOCALE), so in-app links keep the language. */
  localeCookie?: string;
}

export interface Score { runs: number; success: number; rate: number; parity: number | null; avg_steps: number | null; avg_ms: number | null }

export interface Report {
  generated_at: string;
  options: Options;
  tasks: { id: string; goal: string }[];
  runs: Run[];
  scores: Record<string, Score>;
}

const RTL = new Set(["ar", "he", "fa", "ur"]);

export async function loadTasks(path: string): Promise<Task[]> {
  const doc = parse(await readFile(path, "utf8"));
  const tasks: Task[] = Array.isArray(doc) ? doc : doc?.tasks;
  if (!Array.isArray(tasks)) throw new Error(`${path}: expected a list of tasks`);
  for (const t of tasks) {
    if (!t?.id || !t.goal || !t.success) throw new Error(`${path}: each task needs id, goal and success`);
    if (!t.success.url_matches && !t.success.text_visible_key && !t.success.selector) throw new Error(`${path}: task ${t.id} has an empty success condition`);
  }
  return tasks;
}

/** Flat catalogs per locale (missing files are skipped). */
async function loadCatalogs(patterns: string | undefined, locales: string[]): Promise<Record<string, Record<string, string>>> {
  const out: Record<string, Record<string, string>> = {};
  for (const l of locales) {
    out[l] = {};
    for (const p of (patterns ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
      try { flatten(JSON.parse(await readFile(p.replaceAll("{locale}", l), "utf8")), "", out[l]); }
      catch (e) { if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e; }
    }
  }
  return out;
}

export class Ctx {
  indexes: Record<string, Index> = {};
  cat: Record<string, Record<string, string>>;
  source: string;
  constructor(cat: Record<string, Record<string, string>>, source: string) {
    this.cat = cat;
    this.source = source;
    for (const [l, c] of Object.entries(cat)) this.indexes[l] = buildIndex(c);
  }
  msg = (locale: string, key: string) => this.cat[locale]?.[key];
  keyOf = (locale: string, text: string) => match(this.indexes[locale], text);
  /** Other keys whose translation reads the same as `key`'s while their source differs. */
  twins(locale: string, key: string): string[] {
    const m = this.msg(locale, key);
    if (!m) return [];
    return Object.entries(this.cat[locale]).filter(([k, v]) => k !== key && norm(v) === norm(m) && norm(this.msg(this.source, k) ?? "") !== norm(this.msg(this.source, key) ?? "")).map(([k]) => k);
  }
  /** Best source key for a label the agent expected: exact, else most shared words (Jaccard ≥ 0.5). */
  guessKey(sourceText: string): string | undefined {
    const exact = this.keyOf(this.source, sourceText);
    if (exact) return exact;
    const words = (s: string) => new Set(norm(s).split(/[^\p{L}\p{N}]+/u).filter(Boolean));
    const a = words(sourceText);
    if (!a.size) return;
    let best: [string, number] | undefined;
    // ponytail: linear scan of the source catalog per give_up; fine, give_ups are rare.
    for (const [k, v] of Object.entries(this.cat[this.source] ?? {})) {
      const b = words(v);
      const inter = [...a].filter((w) => b.has(w)).length;
      const score = inter / (a.size + b.size - inter);
      if (!best || score > best[1]) best = [k, score];
    }
    return best && best[1] >= 0.5 ? best[0] : undefined;
  }
}

const q = JSON.stringify;

/** Turns a missing/odd label into a finding tied to its key, with a fix. */
export function blame(ctx: Ctx, locale: string, v: View, key: string): Finding {
  const want = ctx.msg(locale, key);
  const src = ctx.msg(ctx.source, key);
  if (locale !== ctx.source && (!want || norm(want) === norm(src ?? "")) && find(v, src).length) {
    return { kind: "untranslated", key, text: src, detail: `shows the source text ${q(src)}`, suggestion: `glot translate --locale ${locale} --key ${key}` };
  }
  if (!want) return { kind: "untranslated", key, detail: `no ${locale} translation and no matching control`, suggestion: `glot translate --locale ${locale} --key ${key}` };
  const shown = v.els.find((e) => e.truncated && want.toLowerCase().startsWith(e.name.toLowerCase().replace(/[.…]+$/, "")));
  if (shown) return { kind: "truncated", key, text: want, detail: `${q(want)} is cut off`, suggestion: `glot fit --locales ${locale}` };
  return { kind: "not_found", key, text: want, detail: `no control labelled ${q(want)}` };
}

/** LLM `give_up`: tie "I looked for X" to the key the agent meant. */
export function attribute(ctx: Ctx, locale: string, g: Extract<Decision, { action: "give_up" }>): Finding {
  const key = ctx.guessKey(g.looked_for_source) ?? ctx.keyOf(locale, g.looked_for);
  const actual = key ? ctx.msg(locale, key) : undefined;
  const f: Finding = { kind: (["not_found", "truncated", "ambiguous", "untranslated", "rtl"].includes(g.reason) ? g.reason : "not_found") as Kind, key, text: actual, detail: g.detail };
  // What a native user expected to read is a candidate label.
  if (key && g.looked_for && norm(actual ?? "") !== norm(g.looked_for)) {
    f.detail = `${actual ? `the control ${q(actual)}` : `\`${key}\` (no ${locale} translation)`} was looked for as ${q(g.looked_for)}. ${g.detail}`.trim();
    f.suggestion = `glot set ${locale} ${key} ${q(g.looked_for)}`;
  }
  return f;
}

async function check(page: Page, ctx: Ctx, locale: string, s: Task["success"], findings: Finding[]): Promise<boolean> {
  if (s.url_matches && !new RegExp(s.url_matches).test(new URL(page.url()).pathname + new URL(page.url()).search)) return false;
  if (s.selector && !(await page.locator(s.selector).first().isVisible().catch(() => false))) return false;
  if (s.text_visible_key) {
    const lines = (await page.locator("body").innerText()).split("\n").map((l) => l.trim()).filter(Boolean);
    const hit = (msg?: string) => !!msg && lines.some((l) => match(buildIndex({ k: msg }), l) === "k");
    const k = s.text_visible_key;
    if (hit(ctx.msg(locale, k))) return true;
    if (locale !== ctx.source && hit(ctx.msg(ctx.source, k))) {
      findings.push({ kind: "untranslated", key: k, text: ctx.msg(ctx.source, k), detail: "confirmation shown in the source language", suggestion: `glot translate --locale ${locale} --key ${k}` });
      return true;
    }
    return false;
  }
  return true;
}

/** Mock step -> concrete action on the current view, or the finding that explains why it can't be done. */
function resolve(ctx: Ctx, locale: string, v: View, step: Step, findings: Finding[]): { action: Action; key?: string; label: string; selector?: string } | Finding {
  if ("scroll" in step) return { action: { action: "scroll", direction: step.scroll }, label: step.scroll };
  const [verb, raw] = Object.entries(step)[0] as ["click" | "type" | "select", Target | string];
  const t: Target & { text?: string; value?: string; option_key?: string } = typeof raw === "string" ? { key: raw } : raw;
  let ref: number | undefined;
  if (t.key) {
    const hits = find(v, ctx.msg(locale, t.key));
    if (!hits.length) return blame(ctx, locale, v, t.key);
    const h = hits[0];
    if (h.truncated) findings.push({ kind: "truncated", key: t.key, text: ctx.msg(locale, t.key), detail: `label cut off: ${q(h.name)}`, suggestion: `glot fit --locales ${locale}` });
    const twins = ctx.twins(locale, t.key).filter((k) => find(v, ctx.msg(ctx.source, k)).length || hits.length > 1);
    if (twins.length) findings.push({ kind: "ambiguous", key: t.key, text: h.name, detail: `${q(h.name)} also translates ${twins.map((k) => `\`${k}\``).join(", ")}`, suggestion: `give \`${t.key}\` a distinct ${locale} label` });
    ref = h.ref;
  }
  const label = t.key ?? t.selector ?? "";
  const action: Action = verb === "click" ? { action: "click", ref: ref ?? -1 } : verb === "type" ? { action: "type", ref: ref ?? -1, text: t.text ?? "" } : { action: "select", ref: ref ?? -1, value: (t.option_key ? ctx.msg(locale, t.option_key) ?? ctx.msg(ctx.source, t.option_key) : t.value) ?? "" };
  return { action, key: t.key, label, selector: ref === undefined ? t.selector : undefined };
}

const slug = (s: string) => s.replace(/[^\w-]+/g, "_");

async function runOne(page: Page, o: Options, ctx: Ctx, task: Task, locale: string, goal: string, client?: Anthropic): Promise<Run> {
  const t0 = Date.now();
  const run: Run = { task: task.id, locale, goal, success: false, steps: 0, ms: 0, findings: [], trace: [] };
  const max = task.max_steps ?? o.maxSteps;
  await page.context().clearCookies();
  if (o.localeCookie) await page.context().addCookies([{ name: o.localeCookie, value: locale, url: o.baseUrl }]);
  await page.goto(buildUrl(o, locale, task.start ?? "/"), { waitUntil: "load" });
  await page.waitForLoadState("networkidle", { timeout: 5000 }).catch(() => {});
  const shot = async (i: number) => {
    const rel = join("screenshots", `${slug(task.id)}_${locale}_${i}.png`);
    const png = await page.screenshot({ path: join(o.out, rel) });
    return { rel, png };
  };

  const first = await observe(page);
  if (RTL.has(locale.split("-")[0]) && first.dir !== "rtl") run.findings.push({ kind: "rtl", detail: `dir="${first.dir}" (expected "rtl")` });

  const agent = client ? new ClaudeAgent(client, o.model, goal, locale, o.sourceLocale) : undefined;
  const script = task.script ?? [];
  if (!agent && !script.length) throw new Error(`task ${task.id}: --provider mock needs a script`);
  let fail: Finding | undefined;

  try {
    for (let i = 0; ; i++) {
      if (await check(page, ctx, locale, task.success, run.findings)) { run.success = true; break; }
      if (!agent && i >= script.length) { fail = { kind: "check_failed", key: task.success.text_visible_key, detail: "script finished, success condition not met" }; break; }
      if (i >= max) { fail = { kind: "max_steps", detail: `no success after ${max} steps` }; break; }
      const v = await observe(page);
      const { rel, png } = await shot(i);
      let action: Action, key: string | undefined, label: string, selector: string | undefined;
      if (agent) {
        const d = await agent.next(v, png);
        if (d.action === "done") {
          fail = (await check(page, ctx, locale, task.success, run.findings)) ? undefined : { kind: "check_failed", key: task.success.text_visible_key, detail: `agent said done (${d.summary}) but the success condition is not met` };
          run.success = !fail;
          run.trace.push({ step: i + 1, action: "done", target: d.summary, screenshot: rel });
          break;
        }
        if (d.action === "give_up") {
          fail = attribute(ctx, locale, d);
          run.trace.push({ step: i + 1, action: "give_up", target: d.looked_for, key: fail.key, screenshot: rel });
          break;
        }
        action = d;
        const el = "ref" in d ? v.els.find((e) => e.ref === d.ref) : undefined;
        label = el?.name ?? ("direction" in d ? d.direction : "");
        key = el ? ctx.keyOf(locale, el.name) : undefined;
        if (el?.truncated && key) run.findings.push({ kind: "truncated", key, text: ctx.msg(locale, key), detail: `label cut off: ${q(el.name)}`, suggestion: `glot fit --locales ${locale}` });
      } else {
        // Users wait for a page to show up: retry for up to 5 s before calling a control missing.
        let r = resolve(ctx, locale, v, script[i], run.findings);
        for (let w = 0; !("action" in r) && w < 20; w++) {
          await page.waitForTimeout(250);
          r = resolve(ctx, locale, await observe(page), script[i], run.findings);
        }
        if (!("action" in r)) { fail = r; run.trace.push({ step: i + 1, action: Object.keys(script[i])[0], target: r.text ?? r.key, key: r.key, screenshot: rel, error: r.detail }); break; }
        ({ action, key, label, selector } = r);
      }
      run.steps = i + 1;
      const entry: Run["trace"][number] = { step: i + 1, action: action.action, target: label, key, screenshot: rel };
      run.trace.push(entry);
      try {
        await act(page, action, selector);
      } catch (e) {
        entry.error = (e as Error).message.split("\n")[0];
        if (!agent) { fail = { kind: "not_found", key, detail: entry.error }; break; }
      }
    }
  } catch (e) {
    fail = { kind: "error", detail: (e as Error).message.split("\n")[0] };
  }
  await shot(run.trace.length).then(({ rel }) => run.trace.push({ step: run.trace.length + 1, action: "final", screenshot: rel }));
  if (!run.success) {
    // "Ran out of steps" says less than a label problem seen on the way.
    const vague = !fail || ["check_failed", "max_steps"].includes(fail.kind);
    const label = vague ? run.findings.find((f) => f.kind !== "rtl") ?? run.findings[0] : undefined;
    if (fail) run.findings.push(fail);
    run.reason = label?.kind ?? fail?.kind ?? "error";
  }
  run.ms = Date.now() - t0;
  return run;
}

async function goals(o: Options, tasks: Task[], client?: Anthropic): Promise<Record<string, string>> {
  const path = join(o.out, "goals.json");
  let cache: Record<string, string> = {};
  try { cache = JSON.parse(await readFile(path, "utf8")); } catch {}
  const out: Record<string, string> = {};
  for (const t of tasks) {
    for (const l of o.locales) {
      const id = `${l}:${createHash("sha256").update(t.goal).digest("hex").slice(0, 16)}`;
      if (l === o.sourceLocale || !client) out[`${t.id}\0${l}`] = t.goal;
      else out[`${t.id}\0${l}`] = cache[id] ??= await translateGoal(client, o.model, t.goal, l);
    }
  }
  await writeFile(path, JSON.stringify(cache, null, 2));
  return out;
}

export function score(runs: Run[], locales: string[], source: string): Record<string, Score> {
  const by = (l: string) => runs.filter((r) => r.locale === l);
  const avg = (xs: number[]) => (xs.length ? Math.round(xs.reduce((a, b) => a + b, 0) / xs.length) : null);
  // Parity only counts tasks the source locale can do: a broken task is not a localization problem.
  const doable = new Set(by(source).filter((r) => r.success).map((r) => r.task));
  const out: Record<string, Score> = {};
  for (const l of locales) {
    const ok = by(l).filter((r) => r.success);
    out[l] = {
      runs: by(l).length,
      success: ok.length,
      rate: by(l).length ? Math.round((ok.length / by(l).length) * 100) : 0,
      parity: doable.size ? Math.round((ok.filter((r) => doable.has(r.task)).length / doable.size) * 100) : null,
      avg_steps: avg(ok.map((r) => r.steps)),
      avg_ms: avg(ok.map((r) => r.ms)),
    };
  }
  return out;
}

/** `client`: injected in tests; otherwise built from the environment for `--provider anthropic`. */
export async function run(o: Options, client?: Anthropic): Promise<Report> {
  if (!o.locales.includes(o.sourceLocale)) o.locales = [o.sourceLocale, ...o.locales];
  const tasks = await loadTasks(o.tasks);
  await mkdir(join(o.out, "screenshots"), { recursive: true });
  const ctx = new Ctx(await loadCatalogs(o.catalogs, o.locales), o.sourceLocale);
  client ??= o.provider === "anthropic" ? new Anthropic({
    // Org-level user keys (sk-ant-usr-…) must name the workspace to bill.
    defaultHeaders: process.env.ANTHROPIC_WORKSPACE_ID ? { "anthropic-workspace-id": process.env.ANTHROPIC_WORKSPACE_ID } : undefined,
  }) : undefined;
  const goal = await goals(o, tasks, client);
  const [width, height] = o.viewport.split("x").map(Number);
  const browser = await chromium.launch();
  const runs: Run[] = [];
  try {
    const page = await browser.newPage({ viewport: { width, height } });
    for (const t of tasks) {
      for (const l of o.locales) {
        const r = await runOne(page, o, ctx, t, l, goal[`${t.id}\0${l}`], client);
        runs.push(r);
        console.error(`${r.success ? "✓" : "✗"} ${t.id} [${l}] ${r.steps} step(s) ${r.ms}ms${r.reason ? ` — ${r.reason}` : ""}`);
      }
    }
  } finally {
    await browser.close();
  }
  const report: Report = { generated_at: new Date().toISOString(), options: o, tasks: tasks.map(({ id, goal }) => ({ id, goal })), runs, scores: score(runs, o.locales, o.sourceLocale) };
  await writeFile(join(o.out, "report.json"), JSON.stringify(report, null, 2));
  await writeFile(join(o.out, "report.md"), toMarkdown(report));
  return report;
}

const cell = (s = "") => s.replace(/\|/g, "\\|").replace(/\s+/g, " ").slice(0, 160);

export function toMarkdown(r: Report): string {
  const L = r.options.locales;
  const lines = ["## glot task parity", "", `| Task | ${L.map((l) => `\`${l}\``).join(" | ")} |`, `|---|${L.map(() => ":---:").join("|")}|`];
  for (const t of r.tasks) {
    lines.push(`| ${t.id} | ${L.map((l) => {
      const x = r.runs.find((y) => y.task === t.id && y.locale === l);
      return !x ? "·" : x.success ? `✅ ${x.steps}` : `❌ ${x.reason}`;
    }).join(" | ")} |`);
  }
  lines.push(`| **Parity** | ${L.map((l) => (r.scores[l].parity === null ? "–" : `**${r.scores[l].parity}%**`)).join(" | ")} |`);
  lines.push("", "✅ n = succeeded in n steps. Parity = share of the tasks the source locale completes that this locale completes too.");
  const findings = r.runs.flatMap((x) => x.findings.map((f) => ({ ...f, task: x.task, locale: x.locale })));
  if (findings.length) {
    lines.push("", "### Findings", "", "| Task | Locale | Kind | Key | Detail | Fix |", "|---|---|---|---|---|---|");
    for (const f of findings) lines.push(`| ${f.task} | \`${f.locale}\` | ${f.kind} | ${f.key ? `\`${f.key}\`` : ""} | ${cell(f.detail)} | ${f.suggestion ? `\`${cell(f.suggestion)}\`` : ""} |`);
  }
  return lines.join("\n") + "\n";
}
