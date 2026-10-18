#!/usr/bin/env node

// src/cli.ts
import { parseArgs } from "node:util";
import { join as join2 } from "node:path";

// src/parity.ts
import Anthropic from "@anthropic-ai/sdk";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { parse } from "yaml";

// ../visual-qa/src/qa.ts
import { chromium } from "playwright";

// ../visual-qa/src/keys.ts
var MARK = /\u2063([\u{FE00}-\u{FE0F}\u{E0100}-\u{E01EF}]+)\u2063/gu;
function decodeKeys(text) {
  const keys = [];
  const clean = text.replace(MARK, (_, bytes) => {
    const b = Array.from(bytes, (c) => {
      const p = c.codePointAt(0);
      return p < 917760 ? p - 65024 : p - 917760 + 16;
    });
    keys.push(new TextDecoder().decode(new Uint8Array(b)));
    return "";
  });
  return { keys, text: clean };
}
var norm = (s) => decodeKeys(s).text.replace(/\s+/g, " ").trim().toLowerCase();
var esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
function closing(s, open) {
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === "{") depth++;
    else if (s[i] === "}" && --depth === 0) return i;
  }
  return -1;
}
function compile(msg, inPlural = false) {
  let re = "", literal = 0, dynamic = false, lit = "";
  const flush = () => {
    if (!lit) return;
    const n = norm(lit);
    literal += n.length;
    re += (/^\s/.test(lit) ? "\\s*" : "") + esc(n).replace(/ /g, "\\s+") + (/\s$/.test(lit) && n ? "\\s*" : "");
    lit = "";
  };
  for (let i = 0; i < msg.length; i++) {
    const c = msg[i];
    if (c === "{") {
      const end = closing(msg, i);
      if (end < 0) {
        lit += msg.slice(i);
        break;
      }
      flush();
      dynamic = true;
      const inner = msg.slice(i + 1, end);
      const m = /^\s*([^,{}]+?)\s*,\s*(plural|select|selectordinal)\s*,([\s\S]*)$/.exec(inner);
      if (m) {
        const branches = [];
        const rest = m[3].replace(/offset:\s*\d+/, "");
        for (let j = 0; j < rest.length; ) {
          const b = rest.indexOf("{", j);
          if (b < 0) break;
          const e = closing(rest, b);
          if (e < 0) break;
          branches.push(compile(rest.slice(b + 1, e), m[2] !== "select"));
          j = e + 1;
        }
        re += `(?:${branches.map((b) => b.re).join("|")})`;
        if (branches.length) literal += Math.min(...branches.map((b) => b.literal));
      } else re += ".+?";
      i = end;
    } else if (c === "#" && inPlural) {
      flush();
      dynamic = true;
      re += ".+?";
    } else if (c === "<") {
      const e = msg.indexOf(">", i);
      if (e < 0) {
        lit += c;
        continue;
      }
      flush();
      if (/^<br\s*\/?>$/i.test(msg.slice(i, e + 1))) re += "\\s*";
      i = e;
    } else lit += c;
  }
  flush();
  return { re: re.replace(/(\\s[*+])+/g, (m) => m.includes("+") ? "\\s+" : "\\s*"), literal, dynamic };
}
function buildIndex(entries) {
  const idx = { exact: /* @__PURE__ */ new Map(), patterns: [] };
  for (const [key, msg] of Object.entries(entries)) {
    if (typeof msg !== "string" || !msg.trim()) continue;
    const c = compile(msg);
    if (!c.dynamic) {
      const n = norm(msg.replace(/<br\s*\/?>/gi, " ").replace(/<\/?[\w.-]+\s*\/?>/g, ""));
      if (!idx.exact.has(n)) idx.exact.set(n, key);
    } else if (c.literal > 0) {
      idx.patterns.push({ key, re: new RegExp(`^${c.re}$`, "su"), literal: c.literal });
    }
  }
  idx.patterns.sort((a, b) => b.literal - a.literal);
  return idx;
}
function match(idx, text) {
  const marked = decodeKeys(text).keys[0];
  if (marked) return marked;
  if (!idx) return;
  const n = norm(text);
  if (!n) return;
  return idx.exact.get(n) ?? idx.patterns.find((p) => p.re.test(n))?.key;
}
function flatten(v2, prefix = "", out = {}) {
  if (typeof v2 === "string") out[prefix] = v2;
  else if (v2 && typeof v2 === "object") {
    for (const [k, x] of Object.entries(v2)) flatten(x, prefix ? `${prefix}.${k}` : k, out);
  }
  return out;
}

// ../visual-qa/src/qa.ts
async function launch() {
  return chromium.launch().catch((e) => {
    if (/Executable doesn't exist/.test(e.message)) throw new Error(`Playwright's Chromium is missing: run \`npx playwright install chromium\`
${e.message.split("\n")[0]}`);
    throw e;
  });
}
function buildUrl(o, locale, route) {
  const pattern = locale === o.sourceLocale ? o.urlPattern.replace(/\/?\{locale\}/, "") : o.urlPattern;
  let path = pattern.replace("{locale}", locale).replace("{route}", route).replace(/\/{2,}/g, "/");
  if (path.length > 1) path = path.replace(/\/$/, "");
  return o.baseUrl.replace(/\/$/, "") + (path.startsWith("/") ? path : "/" + path);
}

// src/agent.ts
function scan() {
  const els = [];
  const q2 = "a[href], button, input:not([type=hidden]), select, textarea, summary, [role=button], [role=tab], [role=link], [role=menuitem], [role=checkbox], [role=radio], [role=switch], [role=option]";
  const text = (e) => {
    if (!e.querySelector("[aria-hidden=true]")) return e.innerText;
    const c = e.cloneNode(true);
    c.querySelectorAll("[aria-hidden=true]").forEach((x) => x.remove());
    return c.textContent ?? "";
  };
  let ref = 0;
  for (const el of Array.from(document.querySelectorAll(q2))) {
    el.removeAttribute("data-glot-ref");
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    if (r.width < 2 || r.height < 2 || cs.visibility === "hidden" || el.closest("[aria-hidden=true], [inert]")) continue;
    const input = el;
    const label = input.labels?.[0]?.innerText;
    const name = (el.getAttribute("aria-label") || text(el) || label || input.placeholder || (input.type === "submit" ? input.value : "") || el.title || "").replace(/\s+/g, " ").trim();
    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute("role") || (tag === "a" ? "link" : tag === "select" ? "combobox" : tag === "textarea" ? "textbox" : tag === "input" ? ["checkbox", "radio", "submit", "button"].includes(input.type) ? input.type : "textbox" : tag);
    const clipped = /hidden|clip/.test(cs.overflowX + cs.overflowY) || cs.textOverflow === "ellipsis";
    el.setAttribute("data-glot-ref", String(++ref));
    els.push({
      ref,
      role,
      name: name.slice(0, 120),
      truncated: clipped && el.scrollWidth > el.clientWidth + 1,
      ...tag === "select" ? { options: Array.from(el.options).map((o) => o.text.trim()) } : {}
    });
    if (ref >= 150) break;
  }
  const texts = Array.from(document.querySelectorAll("h1, h2, h3, [role=alert], [role=status], [aria-live] > *, dialog, [role=dialog] p")).filter((e) => e.getBoundingClientRect().height > 2).map((e) => e.innerText.replace(/\s+/g, " ").trim()).filter(Boolean).slice(0, 40);
  return { url: location.href, els, texts, dir: document.documentElement.dir || document.body.dir || "ltr" };
}
var observe = (page) => page.evaluate(scan);
function render(v2) {
  const lines = [`URL: ${v2.url}`, ...v2.texts.map((t) => `text: ${JSON.stringify(t)}`)];
  for (const e of v2.els) {
    lines.push(`[${e.ref}] ${e.role} ${JSON.stringify(e.name)}${e.truncated ? " (truncated)" : ""}${e.options ? ` options=${JSON.stringify(e.options)}` : ""}`);
  }
  return lines.join("\n");
}
async function act(page, a, selector) {
  const before = page.url();
  if (a.action === "scroll") {
    await page.mouse.wheel(0, a.direction === "down" ? 600 : -600);
  } else {
    const el = page.locator(selector ?? `[data-glot-ref="${a.ref}"]`).first();
    if (selector) await el.waitFor({ state: "visible", timeout: 5e3 }).catch(() => {
    });
    if (a.action === "click") await el.click({ timeout: 3e3 });
    else if (a.action === "type") await el.fill(a.text, { timeout: 3e3 });
    else await el.selectOption({ label: a.value }, { timeout: 3e3 }).catch(() => el.selectOption(a.value, { timeout: 3e3 }));
  }
  if (a.action === "click") await page.waitForURL((u) => u.href !== before, { timeout: 1500, waitUntil: "commit" }).catch(() => {
  });
  await page.waitForLoadState("networkidle", { timeout: 1500 }).catch(() => {
  });
}
function find(v2, msg) {
  if (!msg) return [];
  const idx = buildIndex({ k: msg });
  return v2.els.filter((e) => match(idx, e.name) === "k");
}
var MODEL = "claude-sonnet-5-5";
var TOOLS = [
  { name: "click", description: "Click the element with this ref.", input_schema: { type: "object", properties: { ref: { type: "integer" } }, required: ["ref"], additionalProperties: false } },
  { name: "type", description: "Replace the content of a text field with this text.", input_schema: { type: "object", properties: { ref: { type: "integer" }, text: { type: "string" } }, required: ["ref", "text"], additionalProperties: false } },
  { name: "select", description: "Choose an option (by its visible label) in a dropdown.", input_schema: { type: "object", properties: { ref: { type: "integer" }, value: { type: "string" } }, required: ["ref", "value"], additionalProperties: false } },
  { name: "scroll", description: "Scroll the page.", input_schema: { type: "object", properties: { direction: { type: "string", enum: ["up", "down"] } }, required: ["direction"], additionalProperties: false } },
  { name: "done", description: "The goal is achieved.", input_schema: { type: "object", properties: { summary: { type: "string" } }, required: ["summary"], additionalProperties: false } },
  {
    name: "give_up",
    description: "You cannot achieve the goal. Say what blocked you.",
    input_schema: {
      type: "object",
      properties: {
        reason: { type: "string", enum: ["not_found", "truncated", "ambiguous", "untranslated", "rtl", "other"], description: "not_found: the control you expected is not there; truncated: a label is cut off; ambiguous: several controls could match; untranslated: text in another language; rtl: broken right-to-left layout." },
        looked_for: { type: "string", description: "The label you were looking for, in the UI language, as you expected to read it." },
        looked_for_source: { type: "string", description: "The same label in the app's source language." },
        detail: { type: "string" }
      },
      required: ["reason", "looked_for", "looked_for_source", "detail"],
      additionalProperties: false
    }
  }
].map((t) => ({ ...t, strict: true }));
var ClaudeAgent = class {
  messages = [];
  pending;
  system;
  client;
  model;
  constructor(client, model, goal, locale, source) {
    this.client = client;
    this.model = model;
    this.system = `You are a native "${locale}" speaker using a web app whose UI is in your language. You do not read "${source}". Achieve the user's goal by acting on the page, like a real user would: read labels, do not guess hidden URLs. Each turn you get a screenshot and the list of interactive elements as [ref] role "label". Call exactly one tool per turn. Call done once the goal is visibly achieved, give_up if you are stuck.

Goal: ${goal}`;
  }
  async next(v2, png) {
    const observation = [
      { type: "image", source: { type: "base64", media_type: "image/png", data: png.toString("base64") } },
      { type: "text", text: render(v2) }
    ];
    this.messages.push({
      role: "user",
      content: this.pending ? [{ type: "tool_result", tool_use_id: this.pending, content: observation }] : observation
    });
    const res = await this.client.beta.messages.create({
      model: this.model,
      max_tokens: 8e3,
      system: this.system,
      tools: TOOLS,
      tool_choice: { type: "auto", disable_parallel_tool_use: true },
      output_config: { effort: "medium" },
      cache_control: { type: "ephemeral" },
      betas: ["server-side-fallback-2026-07-01"],
      fallbacks: "default",
      messages: this.messages
    });
    this.messages.push({ role: "assistant", content: res.content });
    if (res.stop_reason === "refusal") throw new Error(`model refused (${res.stop_details?.category ?? "?"})`);
    const call = res.content.find((b) => b.type === "tool_use");
    if (!call) {
      this.pending = void 0;
      const text = res.content.flatMap((b) => b.type === "text" ? [b.text] : []).join(" ");
      return { action: "give_up", reason: "other", looked_for: "", looked_for_source: "", detail: `no tool call: ${text.slice(0, 200)}` };
    }
    this.pending = call.id;
    return { action: call.name, ...call.input };
  }
};
async function translateGoal(client, model, goal, locale) {
  const res = await client.messages.create({
    model,
    max_tokens: 2e3,
    output_config: { effort: "low" },
    messages: [{ role: "user", content: `Translate this task, given to a user of a web app, into the language with locale code "${locale}". Keep names, amounts and dates. Reply with the translation only.

${goal}` }]
  });
  if (res.stop_reason === "refusal") throw new Error("goal translation refused");
  return res.content.flatMap((b) => b.type === "text" ? [b.text] : []).join("").trim();
}

// src/parity.ts
var RTL = /* @__PURE__ */ new Set(["ar", "he", "fa", "ur"]);
async function loadTasks(path) {
  const doc = parse(await readFile(path, "utf8"));
  const tasks = Array.isArray(doc) ? doc : doc?.tasks;
  if (!Array.isArray(tasks)) throw new Error(`${path}: expected a list of tasks`);
  for (const t of tasks) {
    if (!t?.id || !t.goal || !t.success) throw new Error(`${path}: each task needs id, goal and success`);
    if (!t.success.url_matches && !t.success.text_visible_key && !t.success.selector) throw new Error(`${path}: task ${t.id} has an empty success condition`);
  }
  return tasks;
}
async function loadCatalogs(patterns, locales) {
  const out = {};
  for (const l of locales) {
    out[l] = {};
    for (const p of (patterns ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
      try {
        flatten(JSON.parse(await readFile(p.replaceAll("{locale}", l), "utf8")), "", out[l]);
      } catch (e) {
        if (e.code !== "ENOENT") throw e;
      }
    }
  }
  return out;
}
var Ctx = class {
  indexes = {};
  cat;
  source;
  constructor(cat, source) {
    this.cat = cat;
    this.source = source;
    for (const [l, c] of Object.entries(cat)) this.indexes[l] = buildIndex(c);
  }
  msg = (locale, key) => this.cat[locale]?.[key];
  keyOf = (locale, text) => match(this.indexes[locale], text);
  /** Other keys whose translation reads the same as `key`'s while their source differs. */
  twins(locale, key) {
    const m = this.msg(locale, key);
    if (!m) return [];
    return Object.entries(this.cat[locale]).filter(([k, v2]) => k !== key && norm(v2) === norm(m) && norm(this.msg(this.source, k) ?? "") !== norm(this.msg(this.source, key) ?? "")).map(([k]) => k);
  }
  /** Best source key for a label the agent expected: exact, else most shared words (Jaccard ≥ 0.5). */
  guessKey(sourceText) {
    const exact = this.keyOf(this.source, sourceText);
    if (exact) return exact;
    const words = (s) => new Set(norm(s).split(/[^\p{L}\p{N}]+/u).filter(Boolean));
    const a = words(sourceText);
    if (!a.size) return;
    let best;
    for (const [k, v2] of Object.entries(this.cat[this.source] ?? {})) {
      const b = words(v2);
      const inter = [...a].filter((w) => b.has(w)).length;
      const score2 = inter / (a.size + b.size - inter);
      if (!best || score2 > best[1]) best = [k, score2];
    }
    return best && best[1] >= 0.5 ? best[0] : void 0;
  }
};
var q = JSON.stringify;
function blame(ctx, locale, v2, key) {
  const want = ctx.msg(locale, key);
  const src = ctx.msg(ctx.source, key);
  if (locale !== ctx.source && (!want || norm(want) === norm(src ?? "")) && find(v2, src).length) {
    return { kind: "untranslated", key, text: src, detail: `shows the source text ${q(src)}`, suggestion: `glot translate --locale ${locale} --key ${key}` };
  }
  if (!want) return { kind: "untranslated", key, detail: `no ${locale} translation and no matching control`, suggestion: `glot translate --locale ${locale} --key ${key}` };
  const shown = v2.els.find((e) => e.truncated && want.toLowerCase().startsWith(e.name.toLowerCase().replace(/[.…]+$/, "")));
  if (shown) return { kind: "truncated", key, text: want, detail: `${q(want)} is cut off`, suggestion: `glot fit --locales ${locale}` };
  return { kind: "not_found", key, text: want, detail: `no control labelled ${q(want)}` };
}
function attribute(ctx, locale, g) {
  const key = ctx.guessKey(g.looked_for_source) ?? ctx.keyOf(locale, g.looked_for);
  const actual = key ? ctx.msg(locale, key) : void 0;
  const f = { kind: ["not_found", "truncated", "ambiguous", "untranslated", "rtl"].includes(g.reason) ? g.reason : "not_found", key, text: actual, detail: g.detail };
  if (key && g.looked_for && norm(actual ?? "") !== norm(g.looked_for)) {
    f.detail = `${actual ? `the control ${q(actual)}` : `\`${key}\` (no ${locale} translation)`} was looked for as ${q(g.looked_for)}. ${g.detail}`.trim();
    f.suggestion = `glot set ${locale} ${key} ${q(g.looked_for)}`;
  }
  return f;
}
async function check(page, ctx, locale, s, findings) {
  if (s.url_matches && !new RegExp(s.url_matches).test(new URL(page.url()).pathname + new URL(page.url()).search)) return false;
  if (s.selector && !await page.locator(s.selector).first().isVisible().catch(() => false)) return false;
  if (s.text_visible_key) {
    const lines = (await page.locator("body").innerText()).split("\n").map((l) => l.trim()).filter(Boolean);
    const hit = (msg) => !!msg && lines.some((l) => match(buildIndex({ k: msg }), l) === "k");
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
function resolve(ctx, locale, v2, step, findings) {
  if ("scroll" in step) return { action: { action: "scroll", direction: step.scroll }, label: step.scroll };
  const [verb, raw] = Object.entries(step)[0];
  const t = typeof raw === "string" ? { key: raw } : raw;
  let ref;
  if (t.key) {
    const hits = find(v2, ctx.msg(locale, t.key));
    if (!hits.length) return blame(ctx, locale, v2, t.key);
    const h = hits[0];
    if (h.truncated) findings.push({ kind: "truncated", key: t.key, text: ctx.msg(locale, t.key), detail: `label cut off: ${q(h.name)}`, suggestion: `glot fit --locales ${locale}` });
    const twins = ctx.twins(locale, t.key).filter((k) => find(v2, ctx.msg(ctx.source, k)).length || hits.length > 1);
    if (twins.length) findings.push({ kind: "ambiguous", key: t.key, text: h.name, detail: `${q(h.name)} also translates ${twins.map((k) => `\`${k}\``).join(", ")}`, suggestion: `give \`${t.key}\` a distinct ${locale} label` });
    ref = h.ref;
  }
  const label = t.key ?? t.selector ?? "";
  const action = verb === "click" ? { action: "click", ref: ref ?? -1 } : verb === "type" ? { action: "type", ref: ref ?? -1, text: t.text ?? "" } : { action: "select", ref: ref ?? -1, value: (t.option_key ? ctx.msg(locale, t.option_key) ?? ctx.msg(ctx.source, t.option_key) : t.value) ?? "" };
  return { action, key: t.key, label, selector: ref === void 0 ? t.selector : void 0 };
}
async function settle(ok) {
  for (let w = 0; w < 20; w++) {
    await new Promise((r) => setTimeout(r, 250));
    if (await ok()) return true;
  }
  return false;
}
var slug = (s) => s.replace(/[^\w-]+/g, "_");
async function runOne(page, o, ctx, task, locale, goal, client) {
  const t0 = Date.now();
  const run2 = { task: task.id, locale, goal, success: false, steps: 0, ms: 0, findings: [], trace: [] };
  const max = task.max_steps ?? o.maxSteps;
  await page.context().clearCookies();
  const cookies = (o.localeCookie ?? "").split(",").map((s) => s.trim()).filter(Boolean);
  if (cookies.length) await page.context().addCookies(cookies.map((name) => ({ name, value: locale, url: o.baseUrl })));
  await page.goto(buildUrl(o, locale, task.start ?? "/"), { waitUntil: "load" });
  await page.waitForLoadState("networkidle", { timeout: 5e3 }).catch(() => {
  });
  const shot = async (i) => {
    const rel = join("screenshots", `${slug(task.id)}_${locale}_${i}.png`);
    const png = await page.screenshot({ path: join(o.out, rel) });
    return { rel, png };
  };
  const first = await observe(page);
  if (RTL.has(locale.split("-")[0]) && first.dir !== "rtl") run2.findings.push({ kind: "rtl", detail: `dir="${first.dir}" (expected "rtl")` });
  const agent = client ? new ClaudeAgent(client, o.model, goal, locale, o.sourceLocale) : void 0;
  const script = task.script ?? [];
  if (!agent && !script.length) throw new Error(`task ${task.id}: --provider mock needs a script`);
  let fail;
  try {
    for (let i = 0; ; i++) {
      if (await check(page, ctx, locale, task.success, run2.findings)) {
        run2.success = true;
        break;
      }
      if (!agent && i >= script.length && await settle(() => check(page, ctx, locale, task.success, run2.findings))) {
        run2.success = true;
        break;
      }
      if (!agent && i >= script.length) {
        fail = { kind: "check_failed", key: task.success.text_visible_key, detail: "script finished, success condition not met" };
        break;
      }
      if (i >= max) {
        fail = { kind: "max_steps", detail: `no success after ${max} steps` };
        break;
      }
      const v2 = await observe(page);
      const { rel, png } = await shot(i);
      let action, key, label, selector;
      if (agent) {
        const d = await agent.next(v2, png);
        if (d.action === "done") {
          fail = await check(page, ctx, locale, task.success, run2.findings) ? void 0 : { kind: "check_failed", key: task.success.text_visible_key, detail: `agent said done (${d.summary}) but the success condition is not met` };
          run2.success = !fail;
          run2.trace.push({ step: i + 1, action: "done", target: d.summary, screenshot: rel });
          break;
        }
        if (d.action === "give_up") {
          fail = attribute(ctx, locale, d);
          run2.trace.push({ step: i + 1, action: "give_up", target: d.looked_for, key: fail.key, screenshot: rel });
          break;
        }
        action = d;
        const el = "ref" in d ? v2.els.find((e) => e.ref === d.ref) : void 0;
        label = el?.name ?? ("direction" in d ? d.direction : "");
        key = el ? ctx.keyOf(locale, el.name) : void 0;
        if (el?.truncated && key) run2.findings.push({ kind: "truncated", key, text: ctx.msg(locale, key), detail: `label cut off: ${q(el.name)}`, suggestion: `glot fit --locales ${locale}` });
      } else {
        let r = resolve(ctx, locale, v2, script[i], run2.findings);
        for (let w = 0; !("action" in r) && w < 20; w++) {
          await page.waitForTimeout(250);
          r = resolve(ctx, locale, await observe(page), script[i], run2.findings);
        }
        if (!("action" in r)) {
          fail = r;
          run2.trace.push({ step: i + 1, action: Object.keys(script[i])[0], target: r.text ?? r.key, key: r.key, screenshot: rel, error: r.detail });
          break;
        }
        ({ action, key, label, selector } = r);
      }
      run2.steps = i + 1;
      const entry = { step: i + 1, action: action.action, target: label, key, screenshot: rel };
      run2.trace.push(entry);
      try {
        await act(page, action, selector);
      } catch (e) {
        entry.error = e.message.split("\n")[0];
        if (!agent) {
          fail = { kind: "not_found", key, detail: entry.error };
          break;
        }
      }
    }
  } catch (e) {
    fail = { kind: "error", detail: e.message.split("\n")[0] };
  }
  await shot(run2.trace.length).then(({ rel }) => run2.trace.push({ step: run2.trace.length + 1, action: "final", screenshot: rel }));
  if (!run2.success) {
    const vague = !fail || ["check_failed", "max_steps"].includes(fail.kind);
    const label = vague ? run2.findings.find((f) => f.kind !== "rtl") ?? run2.findings[0] : void 0;
    if (fail) run2.findings.push(fail);
    run2.reason = label?.kind ?? fail?.kind ?? "error";
  }
  run2.ms = Date.now() - t0;
  return run2;
}
async function goals(o, tasks, client) {
  const path = join(o.out, "goals.json");
  let cache = {};
  try {
    cache = JSON.parse(await readFile(path, "utf8"));
  } catch {
  }
  const out = {};
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
function score(runs, locales, source) {
  const by = (l) => runs.filter((r) => r.locale === l);
  const avg = (xs) => xs.length ? Math.round(xs.reduce((a, b) => a + b, 0) / xs.length) : null;
  const doable = new Set(by(source).filter((r) => r.success).map((r) => r.task));
  const out = {};
  for (const l of locales) {
    const ok = by(l).filter((r) => r.success);
    out[l] = {
      runs: by(l).length,
      success: ok.length,
      rate: by(l).length ? Math.round(ok.length / by(l).length * 100) : 0,
      parity: doable.size ? Math.round(ok.filter((r) => doable.has(r.task)).length / doable.size * 100) : null,
      avg_steps: avg(ok.map((r) => r.steps)),
      avg_ms: avg(ok.map((r) => r.ms))
    };
  }
  return out;
}
async function run(o, client) {
  if (!o.locales.includes(o.sourceLocale)) o.locales = [o.sourceLocale, ...o.locales];
  const tasks = await loadTasks(o.tasks);
  await mkdir(join(o.out, "screenshots"), { recursive: true });
  const ctx = new Ctx(await loadCatalogs(o.catalogs, o.locales), o.sourceLocale);
  client ??= o.provider === "anthropic" ? new Anthropic({
    // Org-level user keys (sk-ant-usr-…) must name the workspace to bill.
    defaultHeaders: process.env.ANTHROPIC_WORKSPACE_ID ? { "anthropic-workspace-id": process.env.ANTHROPIC_WORKSPACE_ID } : void 0
  }) : void 0;
  const goal = await goals(o, tasks, client);
  const [width, height] = o.viewport.split("x").map(Number);
  const browser = await launch();
  const runs = [];
  try {
    const page = await browser.newPage({ viewport: { width, height } });
    for (const t of tasks) {
      for (const l of o.locales) {
        const r = await runOne(page, o, ctx, t, l, goal[`${t.id}\0${l}`], client);
        runs.push(r);
        console.error(`${r.success ? "\u2713" : "\u2717"} ${t.id} [${l}] ${r.steps} step(s) ${r.ms}ms${r.reason ? ` \u2014 ${r.reason}` : ""}`);
      }
    }
  } finally {
    await browser.close();
  }
  const report2 = { generated_at: (/* @__PURE__ */ new Date()).toISOString(), options: o, tasks: tasks.map(({ id, goal: goal2 }) => ({ id, goal: goal2 })), runs, scores: score(runs, o.locales, o.sourceLocale) };
  await writeFile(join(o.out, "report.json"), JSON.stringify(report2, null, 2));
  await writeFile(join(o.out, "report.md"), toMarkdown(report2));
  return report2;
}
var cell = (s = "") => s.replace(/\|/g, "\\|").replace(/\s+/g, " ").slice(0, 160);
function toMarkdown(r) {
  const L = r.options.locales;
  const lines = ["## glot task parity", "", `| Task | ${L.map((l) => `\`${l}\``).join(" | ")} |`, `|---|${L.map(() => ":---:").join("|")}|`];
  for (const t of r.tasks) {
    lines.push(`| ${t.id} | ${L.map((l) => {
      const x = r.runs.find((y) => y.task === t.id && y.locale === l);
      return !x ? "\xB7" : x.success ? `\u2705 ${x.steps}` : `\u274C ${x.reason}`;
    }).join(" | ")} |`);
  }
  lines.push(`| **Parity** | ${L.map((l) => r.scores[l].parity === null ? "\u2013" : `**${r.scores[l].parity}%**`).join(" | ")} |`);
  lines.push("", "\u2705 n = succeeded in n steps. Parity = share of the tasks the source locale completes that this locale completes too.");
  const findings = r.runs.flatMap((x) => x.findings.map((f) => ({ ...f, task: x.task, locale: x.locale })));
  if (findings.length) {
    lines.push("", "### Findings", "", "| Task | Locale | Kind | Key | Detail | Fix |", "|---|---|---|---|---|---|");
    for (const f of findings) lines.push(`| ${f.task} | \`${f.locale}\` | ${f.kind} | ${f.key ? `\`${f.key}\`` : ""} | ${cell(f.detail)} | ${f.suggestion ? `\`${cell(f.suggestion)}\`` : ""} |`);
  }
  return lines.join("\n") + "\n";
}

// src/cli.ts
var { values: v } = parseArgs({
  options: {
    tasks: { type: "string", default: "glot.tasks.yaml" },
    "base-url": { type: "string", default: "http://localhost:3000" },
    locales: { type: "string", default: "en,fr" },
    "source-locale": { type: "string", default: "en" },
    "url-pattern": { type: "string", default: "/{locale}{route}" },
    // What `glot i18nize` generates: NEXT_LOCALE (Next proxy), glot-locale (React Router / i18next). "" = none.
    "locale-cookie": { type: "string", default: "NEXT_LOCALE,glot-locale" },
    catalogs: { type: "string" },
    provider: { type: "string", default: "anthropic" },
    model: { type: "string", default: MODEL },
    "max-steps": { type: "string", default: "15" },
    viewport: { type: "string", default: "1280x800" },
    out: { type: "string", default: ".glot/parity" },
    "min-parity": { type: "string", default: "0" },
    help: { type: "boolean", short: "h" }
  }
});
if (v.help) {
  console.log(`glot-agent-qa [--tasks glot.tasks.yaml] [--base-url URL] [--locales en,fr,de] [--source-locale en]
  [--url-pattern "/{locale}{route}"] [--locale-cookie NEXT_LOCALE,glot-locale] [--catalogs "messages/{locale}.json"]
  [--provider anthropic|mock] [--model ${MODEL}] [--max-steps 15] [--viewport 1280x800]
  [--out .glot/parity] [--min-parity 90]      (anthropic: ANTHROPIC_API_KEY or an \`ant auth login\` profile)`);
  process.exit(0);
}
if (v.provider !== "mock" && v.provider !== "anthropic") throw new Error("--provider must be mock or anthropic");
var report = await run({
  tasks: v.tasks,
  baseUrl: v["base-url"],
  locales: v.locales.split(",").map((s) => s.trim()).filter(Boolean),
  sourceLocale: v["source-locale"],
  urlPattern: v["url-pattern"],
  localeCookie: v["locale-cookie"],
  catalogs: v.catalogs,
  provider: v.provider,
  model: v.model,
  maxSteps: Number(v["max-steps"]),
  viewport: v.viewport,
  out: v.out
}).catch((e) => {
  console.error(`agent-qa: ${e.message}`);
  process.exit(2);
});
var min = Number(v["min-parity"]);
var below = Object.entries(report.scores).filter(([, s]) => s.parity !== null && s.parity < min);
console.log(Object.entries(report.scores).map(([l, s]) => `${l} ${s.parity ?? "\u2013"}%`).join("  ") + ` \u2014 ${join2(v.out, "report.md")}`);
process.exit(below.length ? 1 : 0);
