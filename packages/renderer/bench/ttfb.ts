// Reproducible TTFB bench: N sequential GETs (fresh connection each, like a first visit), time to the first
// body byte, plus what the Edge said about each response (x-glot-edge state, Server-Timing glot;dur).
//   node bench/ttfb.ts <url> [n] [Header:value …]
//   node bench/ttfb.ts https://cliponit.app/pricing 50 && node bench/ttfb.ts https://es.cliponit.app/pricing 50
import { request as http } from "node:http";
import { request as https } from "node:https";

export interface Sample { ms: number; status: number; state: string; glot: number | null }

export function once(url: string, headers: Record<string, string>): Promise<Sample> {
  const u = new URL(url);
  const req = u.protocol === "https:" ? https : http;
  return new Promise((resolve, reject) => {
    const t0 = performance.now();
    const r = req(u, { headers: { "accept-encoding": "identity", connection: "close", ...headers }, agent: false }, (res) => {
      res.once("data", () => {
        const ms = performance.now() - t0;
        const timing = String(res.headers["server-timing"] ?? "");
        const g = /glot;dur=([\d.]+)/.exec(timing);
        resolve({ ms, status: res.statusCode ?? 0, state: String(res.headers["x-glot-edge"] ?? "-"), glot: g ? Number(g[1]) : null });
        res.resume();
      });
      res.once("error", reject);
    });
    r.once("error", reject);
    r.end();
  });
}

export async function ttfb(url: string, headers: Record<string, string>, n: number): Promise<Sample[]> {
  await once(url, headers); // warm-up (DNS, server JIT, first TM load) not counted
  const out: Sample[] = [];
  for (let i = 0; i < n; i++) out.push(await once(url, headers));
  return out;
}

const pct = (xs: number[], p: number) => { if (!xs.length) return null; const s = [...xs].sort((a, b) => a - b); return Math.round(s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))] * 10) / 10; };
export function summary(s: Sample[]) {
  const states: Record<string, number> = {};
  for (const x of s) states[x.state] = (states[x.state] ?? 0) + 1;
  const glot = s.map((x) => x.glot).filter((g): g is number => g !== null);
  return { p50: pct(s.map((x) => x.ms), 50), p95: pct(s.map((x) => x.ms), 95), glotP50: pct(glot, 50), states: Object.entries(states).map(([k, v]) => `${k}×${v}`).join(", ") };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [url, n = "30", ...hs] = process.argv.slice(2);
  if (!url) { console.error("node bench/ttfb.ts <url> [n] [Header:value …]"); process.exit(2); }
  const headers = Object.fromEntries(hs.map((h) => [h.slice(0, h.indexOf(":")), h.slice(h.indexOf(":") + 1)]));
  console.log(JSON.stringify({ url, n: Number(n), ...summary(await ttfb(url, headers, Number(n))) }));
}
