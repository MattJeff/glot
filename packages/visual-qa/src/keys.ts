// Reverse index: rendered text -> catalog key, without any instrumentation of the app.
import { readFile } from "node:fs/promises";

export const norm = (s: string) => s.replace(/\s+/g, " ").trim().toLowerCase();

const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

function closing(s: string, open: number): number {
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === "{") depth++;
    else if (s[i] === "}" && --depth === 0) return i;
  }
  return -1;
}

/**
 * ICU / i18next message -> regex source matching its rendered text (lowercased, whitespace-collapsed).
 * Placeholders become `.+?`, plural/select become an alternation of their branches, tags are dropped.
 * `literal` counts fixed characters: the most specific match wins.
 */
export function compile(msg: string, inPlural = false): { re: string; literal: number; dynamic: boolean } {
  let re = "", literal = 0, dynamic = false, lit = "";
  const flush = () => {
    if (!lit) return;
    const n = norm(lit);
    literal += n.length;
    // Keep edge whitespace as optional so "Hello {name}" matches "Hello Ada".
    re += (/^\s/.test(lit) ? "\\s*" : "") + esc(n).replace(/ /g, "\\s+") + (/\s$/.test(lit) && n ? "\\s*" : "");
    lit = "";
  };
  for (let i = 0; i < msg.length; i++) {
    const c = msg[i];
    if (c === "{") {
      const end = closing(msg, i);
      if (end < 0) { lit += msg.slice(i); break; }
      flush();
      dynamic = true;
      const inner = msg.slice(i + 1, end);
      const m = /^\s*([^,{}]+?)\s*,\s*(plural|select|selectordinal)\s*,([\s\S]*)$/.exec(inner);
      if (m) {
        const branches: ReturnType<typeof compile>[] = [];
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
      flush(); dynamic = true; re += ".+?";
    } else if (c === "<") {
      const e = msg.indexOf(">", i);
      if (e < 0) { lit += c; continue; }
      flush();
      if (/^<br\s*\/?>$/i.test(msg.slice(i, e + 1))) re += "\\s*";
      i = e;
    } else lit += c;
  }
  flush();
  return { re: re.replace(/(\\s[*+])+/g, (m) => (m.includes("+") ? "\\s+" : "\\s*")), literal, dynamic };
}

export interface Index {
  exact: Map<string, string>;
  patterns: { key: string; re: RegExp; literal: number }[];
}

export function buildIndex(entries: Record<string, string>): Index {
  const idx: Index = { exact: new Map(), patterns: [] };
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
  // ponytail: linear scan over dynamic messages; a prefix trie if catalogs reach many thousands of them.
  idx.patterns.sort((a, b) => b.literal - a.literal);
  return idx;
}

export function match(idx: Index | undefined, text: string): string | undefined {
  if (!idx) return;
  const n = norm(text);
  if (!n) return;
  return idx.exact.get(n) ?? idx.patterns.find((p) => p.re.test(n))?.key;
}

export function flatten(v: unknown, prefix = "", out: Record<string, string> = {}): Record<string, string> {
  if (typeof v === "string") out[prefix] = v;
  else if (v && typeof v === "object") {
    for (const [k, x] of Object.entries(v)) flatten(x, prefix ? `${prefix}.${k}` : k, out);
  }
  return out;
}

/** `patterns`: comma-separated paths with `{locale}`, e.g. `messages/{locale}.json`. Missing files are skipped. */
export async function loadIndexes(patterns: string, locales: string[]): Promise<Record<string, Index>> {
  const out: Record<string, Index> = {};
  for (const locale of locales) {
    const entries: Record<string, string> = {};
    for (const p of patterns.split(",").map((s) => s.trim()).filter(Boolean)) {
      try {
        flatten(JSON.parse(await readFile(p.replaceAll("{locale}", locale), "utf8")), "", entries);
      } catch (e) {
        if ((e as NodeJS.ErrnoException).code !== "ENOENT") throw e;
      }
    }
    out[locale] = buildIndex(entries);
  }
  return out;
}
