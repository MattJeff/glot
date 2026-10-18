#!/usr/bin/env node
import { parseArgs } from "node:util";
import { join } from "node:path";
import { run } from "./qa.ts";

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
    "fail-on": { type: "string", default: "overflow" },
    help: { type: "boolean", short: "h" },
  },
});

if (v.help) {
  console.log(`glot-visual-qa [--base-url URL] [--routes /,/login] [--locales fr,de] [--source-locale en]
  [--url-pattern "/{locale}{route}"] [--viewports 1280x800,390x844] [--out .glot/visual-qa]
  [--suggest] [--fail-on overflow,clamp,overlap,untranslated,tofu,rtl|none]`);
  process.exit(0);
}

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
});

const failOn = new Set(list(v["fail-on"]!));
const failing = report.issues.filter((i) => failOn.has(i.kind));
console.log(`${report.issues.length} issue(s), ${failing.length} failing — ${join(v.out!, "report.md")}`);
process.exit(failing.length ? 1 : 0);
