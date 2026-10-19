import { readFile } from "node:fs/promises";
import { join } from "node:path";
import type { Report } from "./qa.ts";

/**
 * Sends report.json + every screenshot/crop it references to glot-server
 * (`POST /api/projects/{slug}/visual-reports`, JSON with base64 files). Token: GLOT_TOKEN.
 */
export async function upload(o: { server: string; project: string; out: string; label?: string }): Promise<string> {
  const token = process.env.GLOT_TOKEN;
  if (!token) throw new Error("--upload: GLOT_TOKEN is not set (create one in the dashboard under API tokens)");
  const report = JSON.parse(await readFile(join(o.out, "report.json"), "utf8")) as Report;
  const paths = new Set<string>();
  for (const p of report.pages) paths.add(p.screenshot);
  for (const i of report.issues) if (i.crop) paths.add(i.crop);
  const files: Record<string, string> = {};
  for (const p of paths) {
    // Windows path separators would not resolve on the server.
    files[p.replaceAll("\\", "/")] = (await readFile(join(o.out, p))).toString("base64");
  }
  const toPosix = (s: string) => s.replaceAll("\\", "/");
  report.pages = report.pages.map((p) => ({ ...p, screenshot: toPosix(p.screenshot) }));
  report.issues = report.issues.map((i) => (i.crop ? { ...i, crop: toPosix(i.crop) } : i));

  const url = `${o.server.replace(/\/$/, "")}/api/projects/${encodeURIComponent(o.project)}/visual-reports`;
  const res = await fetch(url, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify({ report, files, label: o.label }),
  });
  if (!res.ok) {
    const text = await res.text();
    if (res.status === 404 && text.includes("project not found"))
      throw new Error(`--upload: project "${o.project}" not found on ${o.server}, check the slug in Settings (or the [server] project in glot.toml)`);
    throw new Error(`--upload: HTTP ${res.status} ${text}`);
  }
  const { id } = (await res.json()) as { id: string };
  return id;
}
