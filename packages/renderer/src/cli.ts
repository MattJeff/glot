#!/usr/bin/env node
// glot-renderer in CI ("prerender in the build", no browser on the glot server):
//   glot-renderer prerender --base-url http://localhost:4173 --routes /,/pricing [--mount append] --out dir
// Same engine as the service (src/server.ts): one headless shell, one page at a time, images/fonts/analytics
// blocked, strict timeouts. Writes <out>/<route>/index.html and <out>/manifest.json
// ({"snapshots": [{"path", "file", "renderedAt", "ms"}], "failed": [{"path", "error"}]}), the input of
// `glot prerender --upload` → POST /edge/v1/{project}/snapshots. Exit 1 if any route failed.
// Private hosts are allowed here: CI renders its own preview server (localhost).
import { parseArgs } from "node:util";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

import type { Mount } from "./server.ts";

process.env.RENDERER_ALLOW_PRIVATE = "1"; // read when server.ts loads: import it after
const { render, stopBrowser } = await import("./server.ts");

/** `/pricing/` → `pricing/index.html`, `/` → `index.html`; null for routes that cannot be a file. */
export function routeFile(route: string): string | null {
  if (!route.startsWith("/") || /[?#\\]/.test(route)) return null;
  const parts = route.split("/").filter(Boolean);
  if (parts.some((p) => p === "." || p === "..")) return null;
  return join(...parts, "index.html");
}

export async function prerender(baseUrl: string, routes: string[], out: string, mount: Mount = "replace") {
  const snapshots: { path: string; file: string; renderedAt: number; ms: number }[] = [];
  const failed: { path: string; error: string }[] = [];
  try {
    for (const path of routes) {
      const file = routeFile(path);
      if (!file) { failed.push({ path, error: "route must be a path like /pricing (no query, no ..)" }); continue; }
      const t0 = performance.now();
      try {
        const html = await render(new URL(path, baseUrl).href, mount);
        await mkdir(dirname(join(out, file)), { recursive: true });
        await writeFile(join(out, file), html);
        const ms = Math.round(performance.now() - t0);
        snapshots.push({ path, file, renderedAt: Date.now(), ms });
        console.error(`rendered ${path} (${ms} ms)`);
      } catch (e) {
        failed.push({ path, error: String((e as Error)?.message ?? e) });
        console.error(`failed ${path}: ${failed.at(-1)!.error}`);
      }
    }
  } finally {
    await stopBrowser();
  }
  await mkdir(out, { recursive: true });
  await writeFile(join(out, "manifest.json"), JSON.stringify({ baseUrl, mount, snapshots, failed }, null, 2));
  return { snapshots, failed };
}

if (/cli\.[jt]s$/.test(process.argv[1] ?? "")) {
  const [cmd, ...rest] = process.argv.slice(2);
  const { values } = parseArgs({ args: rest, options: { "base-url": { type: "string" }, routes: { type: "string" }, out: { type: "string" }, mount: { type: "string" } } });
  if (cmd !== "prerender" || !values["base-url"] || !values.routes || !values.out) {
    console.error("usage: glot-renderer prerender --base-url URL --routes /,/pricing --out DIR [--mount append]");
    process.exit(2);
  }
  const routes = values.routes.split(",").map((r) => r.trim()).filter(Boolean);
  const r = await prerender(values["base-url"], routes, values.out, values.mount === "append" ? "append" : "replace");
  process.exit(r.failed.length ? 1 : 0);
}
