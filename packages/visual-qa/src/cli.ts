#!/usr/bin/env node
import { parseArgs } from "node:util";
import { join } from "node:path";
import { run } from "./qa.ts";
import { upload } from "./upload.ts";

const { values: v } = parseArgs({
  options: {
    "base-url": { type: "string", default: "http://localhost:3100" },
    routes: { type: "string", default: "/,/login,/signup" },
    locales: { type: "string", default: "fr,de,ja,ar" },
    "source-locale": { type: "string", default: "en" },
    "url-pattern": { type: "string", default: "/{locale}{route}" },
    viewports: { type: "string", default: "1280x800,390x844" },
    out: { type: "string", default: ".glot/visual-qa" },
    suggest: { type: "boolean", default: false },
    catalogs: { type: "string" },
    map: { type: "string" },
    measure: { type: "boolean", default: false },
    "fonts-dir": { type: "string" },
    "fail-on": { type: "string", default: "overflow" },
    upload: { type: "string" },
    project: { type: "string" },
    label: { type: "string" },
    harness: { type: "string" },
    help: { type: "boolean", short: "h" },
  },
});

if (v.help) {
  console.log(`glot-visual-qa [--base-url URL] [--routes /,/login] [--locales fr,de] [--source-locale en]
  [--url-pattern "/{locale}{route}"] [--viewports 1280x800,390x844] [--out .glot/visual-qa]
  [--suggest] [--catalogs "messages/{locale}.json"] [--map "*"|key1,key2] [--measure [--fonts-dir .glot/fonts]] [--fail-on overflow,clamp,overlap,untranslated,tofu,rtl|none]
  [--upload <glot-server-url> --project <slug> [--label "commit abc123"]]   (token: GLOT_TOKEN)
  [--harness fragments.json]   (glot coverage --render: mount {"<locale>": "<html>"} on the base-url page)`);
  process.exit(0);
}

// ponytail: --locales "" means source pages only (used by `glot translate --visual`).
const list = (s: string) => s.split(",").map((x) => x.trim()).filter(Boolean);
const report = await run({
  baseUrl: v["base-url"]!,
  routes: list(v.routes!),
  locales: list(v.locales!),
  sourceLocale: v["source-locale"]!,
  urlPattern: v["url-pattern"]!,
  viewports: list(v.viewports!),
  out: v.out!,
  suggest: v.suggest!,
  catalogs: v.catalogs,
  map: v.map,
  measure: v.measure,
  fontsDir: v["fonts-dir"],
  harness: v.harness,
});

if (v.upload) {
  if (!v.project) throw new Error("--upload needs --project <slug>");
  const id = await upload({ server: v.upload, project: v.project, out: v.out!, label: v.label });
  console.log(`uploaded visual report ${id} to ${v.upload} (${v.project})`);
}

const failOn = new Set(list(v["fail-on"]!));
const failing = report.issues.filter((i) => failOn.has(i.kind));
console.log(`${report.issues.length} issue(s), ${failing.length} failing — ${join(v.out!, "report.md")}`);
process.exit(failing.length ? 1 : 0);
