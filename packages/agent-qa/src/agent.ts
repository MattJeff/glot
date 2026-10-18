// What the agent sees (reduced accessibility tree + screenshot) and what it can do (click/type/select/scroll).
import type { Page } from "playwright";
import Anthropic from "@anthropic-ai/sdk";
import { buildIndex, match, norm } from "../../visual-qa/src/keys.ts";

export interface El {
  ref: number;
  role: string;
  name: string;
  /** Label clipped by its box (ellipsis / overflow hidden). */
  truncated: boolean;
  options?: string[];
}

export interface View {
  url: string;
  els: El[];
  /** Headings, alerts, live regions: what a user reads to know where they are. */
  texts: string[];
  dir: string;
}

/** Runs inside the page. Must stay self-contained (serialized by Playwright). */
function scan(): View {
  const els: El[] = [];
  const q = "a[href], button, input:not([type=hidden]), select, textarea, summary, [role=button], [role=tab], [role=link], [role=menuitem], [role=checkbox], [role=radio], [role=switch], [role=option]";
  // Accessible name: aria-hidden decorations (icons, emoji) are not part of a link's label.
  const text = (e: HTMLElement) => {
    if (!e.querySelector("[aria-hidden=true]")) return e.innerText;
    const c = e.cloneNode(true) as HTMLElement;
    c.querySelectorAll("[aria-hidden=true]").forEach((x) => x.remove());
    return c.textContent ?? "";
  };
  let ref = 0;
  for (const el of Array.from(document.querySelectorAll<HTMLElement>(q))) {
    el.removeAttribute("data-glot-ref");
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    if (r.width < 2 || r.height < 2 || cs.visibility === "hidden" || el.closest("[aria-hidden=true], [inert]")) continue;
    const input = el as HTMLInputElement;
    const label = input.labels?.[0]?.innerText;
    const name = (el.getAttribute("aria-label") || text(el) || label || input.placeholder || (input.type === "submit" ? input.value : "") || el.title || "").replace(/\s+/g, " ").trim();
    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute("role") || (tag === "a" ? "link" : tag === "select" ? "combobox" : tag === "textarea" ? "textbox" : tag === "input" ? (["checkbox", "radio", "submit", "button"].includes(input.type) ? input.type : "textbox") : tag);
    const clipped = /hidden|clip/.test(cs.overflowX + cs.overflowY) || cs.textOverflow === "ellipsis";
    el.setAttribute("data-glot-ref", String(++ref));
    els.push({
      ref, role, name: name.slice(0, 120),
      truncated: clipped && el.scrollWidth > el.clientWidth + 1,
      ...(tag === "select" ? { options: Array.from((el as HTMLSelectElement).options).map((o) => o.text.trim()) } : {}),
    });
    if (ref >= 150) break; // ponytail: big pages get the first 150 controls; scroll reveals the rest.
  }
  const texts = Array.from(document.querySelectorAll<HTMLElement>("h1, h2, h3, [role=alert], [role=status], [aria-live] > *, dialog, [role=dialog] p"))
    .filter((e) => e.getBoundingClientRect().height > 2)
    .map((e) => e.innerText.replace(/\s+/g, " ").trim())
    .filter(Boolean)
    .slice(0, 40);
  return { url: location.href, els, texts, dir: document.documentElement.dir || document.body.dir || "ltr" };
}

export const observe = (page: Page): Promise<View> => page.evaluate(scan);

/** The tree as the model reads it: `[3] button "Envoyer un rappel" (truncated)`. */
export function render(v: View): string {
  const lines = [`URL: ${v.url}`, ...v.texts.map((t) => `text: ${JSON.stringify(t)}`)];
  for (const e of v.els) {
    lines.push(`[${e.ref}] ${e.role} ${JSON.stringify(e.name)}${e.truncated ? " (truncated)" : ""}${e.options ? ` options=${JSON.stringify(e.options)}` : ""}`);
  }
  return lines.join("\n");
}

export type Action =
  | { action: "click"; ref: number }
  | { action: "type"; ref: number; text: string }
  | { action: "select"; ref: number; value: string }
  | { action: "scroll"; direction: "up" | "down" };

/** `selector` (scripted steps) overrides the ref. */
export async function act(page: Page, a: Action, selector?: string): Promise<void> {
  const before = page.url();
  if (a.action === "scroll") {
    await page.mouse.wheel(0, a.direction === "down" ? 600 : -600);
  } else {
    const el = page.locator(selector ?? `[data-glot-ref="${a.ref}"]`).first();
    if (selector) await el.waitFor({ state: "visible", timeout: 5000 }).catch(() => {});
    if (a.action === "click") await el.click({ timeout: 3000 });
    else if (a.action === "type") await el.fill(a.text, { timeout: 3000 });
    else await el.selectOption({ label: a.value }, { timeout: 3000 }).catch(() => el.selectOption(a.value, { timeout: 3000 }));
  }
  // A client-side navigation (Next <Link>, router.push) starts after the click returns and fires no
  // "load": wait for the URL to change, then for fetches to settle (dev servers never fully idle).
  if (a.action === "click") await page.waitForURL((u) => u.href !== before, { timeout: 1500, waitUntil: "commit" }).catch(() => {});
  await page.waitForLoadState("networkidle", { timeout: 1500 }).catch(() => {});
}

/** Visible elements whose label renders `msg` (ICU placeholders/plurals allowed). */
export function find(v: View, msg: string | undefined): El[] {
  if (!msg) return [];
  const idx = buildIndex({ k: msg });
  return v.els.filter((e) => match(idx, e.name) === "k");
}

export { norm };

// ───────────────────────────────── Claude ─────────────────────────────────

export const MODEL = "claude-sonnet-5-5";

const TOOLS: Anthropic.Beta.BetaTool[] = [
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
        detail: { type: "string" },
      },
      required: ["reason", "looked_for", "looked_for_source", "detail"],
      additionalProperties: false,
    },
  },
].map((t) => ({ ...t, strict: true }) as Anthropic.Beta.BetaTool);

export type Decision =
  | Action
  | { action: "done"; summary: string }
  | { action: "give_up"; reason: string; looked_for: string; looked_for_source: string; detail: string };

/** One agent per (task, locale). Append-only history (thinking blocks stay valid, the prefix stays cached). */
export class ClaudeAgent {
  private messages: Anthropic.Beta.BetaMessageParam[] = [];
  private pending?: string;
  private system: string;
  private client: Anthropic;
  private model: string;
  constructor(client: Anthropic, model: string, goal: string, locale: string, source: string) {
    this.client = client;
    this.model = model;
    this.system = `You are a native "${locale}" speaker using a web app whose UI is in your language. You do not read "${source}". Achieve the user's goal by acting on the page, like a real user would: read labels, do not guess hidden URLs. Each turn you get a screenshot and the list of interactive elements as [ref] role "label". Call exactly one tool per turn. Call done once the goal is visibly achieved, give_up if you are stuck.\n\nGoal: ${goal}`;
  }

  async next(v: View, png: Buffer): Promise<Decision> {
    const observation: Anthropic.Beta.BetaContentBlockParam[] = [
      { type: "image", source: { type: "base64", media_type: "image/png", data: png.toString("base64") } },
      { type: "text", text: render(v) },
    ];
    this.messages.push({
      role: "user",
      content: this.pending ? [{ type: "tool_result", tool_use_id: this.pending, content: observation as Anthropic.Beta.BetaToolResultBlockParam["content"] }] : observation,
    });
    const res = await this.client.beta.messages.create({
      model: this.model,
      max_tokens: 8000,
      system: this.system,
      tools: TOOLS,
      tool_choice: { type: "auto", disable_parallel_tool_use: true },
      output_config: { effort: "medium" },
      cache_control: { type: "ephemeral" },
      betas: ["server-side-fallback-2026-07-01"],
      fallbacks: "default",
      messages: this.messages,
    });
    this.messages.push({ role: "assistant", content: res.content as Anthropic.Beta.BetaContentBlockParam[] });
    if (res.stop_reason === "refusal") throw new Error(`model refused (${res.stop_details?.category ?? "?"})`);
    const call = res.content.find((b): b is Anthropic.Beta.BetaToolUseBlock => b.type === "tool_use");
    if (!call) {
      this.pending = undefined;
      const text = res.content.flatMap((b) => (b.type === "text" ? [b.text] : [])).join(" ");
      return { action: "give_up", reason: "other", looked_for: "", looked_for_source: "", detail: `no tool call: ${text.slice(0, 200)}` };
    }
    this.pending = call.id;
    return { action: call.name, ...(call.input as object) } as Decision;
  }
}

/** Goal in the tested locale, so the agent behaves like a native user. */
export async function translateGoal(client: Anthropic, model: string, goal: string, locale: string): Promise<string> {
  const res = await client.messages.create({
    model,
    max_tokens: 2000,
    output_config: { effort: "low" },
    messages: [{ role: "user", content: `Translate this task, given to a user of a web app, into the language with locale code "${locale}". Keep names, amounts and dates. Reply with the translation only.\n\n${goal}` }],
  });
  if (res.stop_reason === "refusal") throw new Error("goal translation refused");
  return res.content.flatMap((b) => (b.type === "text" ? [b.text] : [])).join("").trim();
}
