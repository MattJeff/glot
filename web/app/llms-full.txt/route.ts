import { readFileSync } from "node:fs";
import path from "node:path";
import { SLUGS } from "@/app/[locale]/docs/content";

// llms-full.txt: llms.txt followed by every docs page, so an AI reader gets the whole documentation in one fetch.
export const dynamic = "force-static";

export function GET() {
  const root = process.cwd();
  const parts = [readFileSync(path.join(root, "public", "llms.txt"), "utf8")];
  for (const slug of SLUGS) {
    parts.push(`\n\n---\n\nSource: https://glotlang.com/docs/${slug}\n\n${readFileSync(path.join(root, "content", "docs", `${slug}.md`), "utf8")}`);
  }
  return new Response(parts.join(""), { headers: { "content-type": "text/plain; charset=utf-8" } });
}
