// Application paths (login, account, payment, OAuth, API…): pages that stay on the origin's own URL, never
// translated, cached or discovered. Same algorithm as crates/server/src/edge/app_paths.rs (shared vectors:
// test/fixtures/app-paths.json). Rules come from `GET /edge/v1/{project}/status` → `app`.
//   `/login` (default action) · `redirect /dashboard` · `proxy /api/`
//   `/explainer` = itself and `/explainer/…` (never `/explainer-video`) · `/live/` = children only · `*` = any run.

export type AppAction = "redirect" | "proxy";
export interface AppRules {
  rules: string[];
  action?: AppAction;
  /** `query:lang` | `prefix` | `none` (from the project's preview url_pattern). */
  locale?: string;
}
export type AppDecision = { kind: "proxy" } | { kind: "redirect"; status: number; location: string; cookie: string };

export const HOP_COOKIE = "glot_app_hop";

/** One rule line → [action, lowercase pattern]; undefined for blank / `#` / invalid lines. */
export function parseLine(line: string, def: AppAction): [AppAction, string] | undefined {
  const l = line.trim();
  if (!l || l.startsWith("#")) return undefined;
  const t = l.split(/\s+/);
  const a = t[0].toLowerCase();
  const [action, p, extra] = a === "redirect" || a === "proxy" ? [a as AppAction, t[1], t[2]] : [def, t[0], t[1]];
  if (!p?.startsWith("/") || extra !== undefined || p.length > 200) return undefined;
  return [action, p.toLowerCase()];
}

const unreserved = (c: number) => (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 45 || c === 46 || c === 95 || c === 126;
const ALLOWED = new Set([..."/!$&'()*+,;=:@"].map((c) => c.charCodeAt(0)));
const hex2 = (v: number) => "%" + v.toString(16).toUpperCase().padStart(2, "0");

/** Normalized path: starts with `/`, ASCII only, no `//`, `.` or `..` segments. */
export function normalize(path: string): string {
  const b = new TextEncoder().encode(path);
  const hx = (c: number | undefined) => (c === undefined ? NaN : parseInt(String.fromCharCode(c), 16));
  let s = "";
  for (let i = 0; i < b.length; i++) {
    const c = b[i];
    if (c === 37) {
      const h = hx(b[i + 1]), l = hx(b[i + 2]);
      if (!Number.isNaN(h) && !Number.isNaN(l)) {
        const v = h * 16 + l;
        s += unreserved(v) ? String.fromCharCode(v) : hex2(v);
        i += 2;
        continue;
      }
    }
    s += c === 92 ? "/" : unreserved(c) || ALLOWED.has(c) ? String.fromCharCode(c) : hex2(c);
  }
  const trailing = s.endsWith("/") || s.endsWith("/.") || s.endsWith("/..");
  const segs: string[] = [];
  for (const seg of s.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") segs.pop();
    else segs.push(seg);
  }
  return "/" + segs.join("/") + (trailing && segs.length ? "/" : "");
}

function wild(p: string, s: string): boolean {
  let pi = 0, si = 0, star = -1, mark = 0;
  while (si < s.length) {
    if (pi < p.length && p[pi] === "*") (star = pi++), (mark = si);
    else if (pi < p.length && p[pi] === s[si]) pi++, si++;
    else if (star >= 0) (pi = star + 1), (si = ++mark);
    else return false;
  }
  while (pi < p.length && p[pi] === "*") pi++;
  return pi === p.length;
}

function matches(pattern: string, path: string): boolean {
  if (pattern === "/") return true;
  if (pattern.includes("*")) return wild(pattern, path);
  if (pattern.endsWith("/")) return path.length > pattern.length && path.startsWith(pattern);
  return path === pattern || path.startsWith(pattern + "/");
}

/** The action of the first rule matching `path` (no query). glot's own files are never app paths. */
export function classify(rules: readonly string[], def: AppAction, path: string): AppAction | undefined {
  const p = normalize(path).toLowerCase();
  if (p === "/sitemap.xml" || p === "/robots.txt" || p.startsWith("/__glot/")) return undefined;
  for (const l of rules) {
    const r = parseLine(l, def);
    if (r && matches(r[1], p)) return r[0];
  }
  return undefined;
}

export interface AppInput {
  rules: readonly string[];
  action: AppAction;
  locale_style: string;
  locale: string;
  /** Where the source pages live (scheme://host[:port] is kept, nothing else). */
  origin: string;
  /** This request's host and public path (with the locale prefix, if any). */
  host: string;
  req_path: string;
  /** Origin path (locale prefix removed) and raw query (no `?`). */
  path: string;
  query: string;
  method: string;
  cookie: string;
}

export function decide(i: AppInput): AppDecision | undefined {
  const action = classify(i.rules, i.action, i.path);
  if (!action) return undefined;
  if (action === "proxy") return { kind: "proxy" };
  let base: URL;
  try {
    base = new URL(i.origin);
  } catch {
    return { kind: "proxy" };
  }
  const norm = normalize(i.path);
  const hop = norm.replace(/[^A-Za-z0-9/._~%-]/g, "_");
  const bounced = i.cookie.split(";").some((raw) => {
    const kv = raw.trim();
    const at = kv.indexOf("=");
    return at > 0 && kv.slice(0, at).trim() === HOP_COOKIE && kv.slice(at + 1) === hop;
  });
  const target = i.locale_style === "prefix" ? `/${i.locale}${norm}` : norm;
  let q = i.query.split("&").filter(Boolean);
  if (i.locale_style.startsWith("query:")) {
    const param = i.locale_style.slice(6);
    q = q.filter((kv) => kv.split("=")[0] !== param);
    q.push(`${param}=${i.locale}`);
  }
  // The visitor came back from the origin, or the target is this very URL: redirecting would loop.
  if (bounced || (base.host.toLowerCase() === i.host.toLowerCase() && target === normalize(i.req_path))) return { kind: "proxy" };
  const get = /^(get|head)$/i.test(i.method);
  return {
    kind: "redirect",
    status: get ? 302 : 307,
    location: base.origin + target + (q.length ? "?" + q.join("&") : ""),
    cookie: `${HOP_COOKIE}=${hop}; Max-Age=30; Path=/; HttpOnly; Secure; SameSite=Lax`,
  };
}
