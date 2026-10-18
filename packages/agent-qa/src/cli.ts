#!/usr/bin/env node
import { parseArgs } from "node:util";
import { join } from "node:path";
import { run } from "./parity.ts";
import { MODEL } from "./agent.ts";
import { main as panel } from "./panel.ts";

// `glot labs panel`: capture only (screenshots, text, CTAs); the personas run in glot.
if (process.argv[2] === "panel") {
  await panel(process.argv.slice(3)).catch((e: Error) => {
    console.error(`agent-qa panel: ${e.message}`);
    process.exit(2);
  });
  process.exit(0);
}

const { values: v } = parseArgs({
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
    help: { type: "boolean", short: "h" },
  },
});

if (v.help) {
  console.log(`glot-agent-qa [--tasks glot.tasks.yaml] [--base-url URL] [--locales en,fr,de] [--source-locale en]
  [--url-pattern "/{locale}{route}"] [--locale-cookie NEXT_LOCALE,glot-locale] [--catalogs "messages/{locale}.json"]
  [--provider anthropic|mock] [--model ${MODEL}] [--max-steps 15] [--viewport 1280x800]
  [--out .glot/parity] [--min-parity 90]      (anthropic: ANTHROPIC_API_KEY or an \`ant auth login\` profile)
glot-agent-qa panel [--config glot.panel.yaml] [--routes /,/pricing] [--base-url URL] [--locales en,fr] [--out .glot/panel]   (capture for \`glot labs panel\`)`);
  process.exit(0);
}

if (v.provider !== "mock" && v.provider !== "anthropic") throw new Error("--provider must be mock or anthropic");
// Exit 1 = parity below --min-parity; 2 = could not run.
const report = await run({
  tasks: v.tasks!,
  baseUrl: v["base-url"]!,
  locales: v.locales!.split(",").map((s) => s.trim()).filter(Boolean),
  sourceLocale: v["source-locale"]!,
  urlPattern: v["url-pattern"]!,
  localeCookie: v["locale-cookie"],
  catalogs: v.catalogs,
  provider: v.provider,
  model: v.model!,
  maxSteps: Number(v["max-steps"]),
  viewport: v.viewport!,
  out: v.out!,
}).catch((e: Error) => {
  console.error(`agent-qa: ${e.message}`);
  process.exit(2);
});

const min = Number(v["min-parity"]);
const below = Object.entries(report.scores).filter(([, s]) => s.parity !== null && s.parity < min);
console.log(Object.entries(report.scores).map(([l, s]) => `${l} ${s.parity ?? "–"}%`).join("  ") + ` — ${join(v.out!, "report.md")}`);
process.exit(below.length ? 1 : 0);
