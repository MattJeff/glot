import { readFileSync } from "node:fs";
import path from "node:path";
import { Marked, type Token } from "marked";
import { GITHUB_URL } from "@/lib/snippets";

// Docs content is English Markdown in web/content/docs/<slug>.md (content, not UI: not in the catalogs).
// Page and section titles shown in the chrome come from messages (`docs.pages.*`, `docs.sections.*`).
export const NAV = [
  { section: "start", pages: ["getting-started", "concepts"] },
  { section: "agents", pages: ["mcp", "integrations", "skills"] },
  { section: "guides", pages: ["frameworks", "runtime", "quality", "provenance"] },
  { section: "reference", pages: ["cli", "faq", "benchmark"] },
  { section: "labs", pages: ["labs", "insights", "self-hosting", "deploy-hetzner", "mcp-remote"] },
] as const;

export type Slug = (typeof NAV)[number]["pages"][number];
export const SLUGS: Slug[] = NAV.flatMap((s) => [...s.pages]);

const DIR = path.join(process.cwd(), "content", "docs");

export function editUrl(slug: Slug) {
  // cli.md is generated: point at its generator instead.
  const file = slug === "cli" ? "web/scripts/gen-cli-docs.ts" : `web/content/docs/${slug}.md`;
  return `${GITHUB_URL}/edit/main/${file}`;
}

export function slugify(text: string) {
  return text
    .toLowerCase()
    .replace(/<[^>]+>|[`*_]/g, "")
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");
}

const marked = new Marked({
  gfm: true,
  renderer: {
    heading({ tokens, depth, text }) {
      const id = slugify(text);
      return `<h${depth} id="${id}"><a href="#${id}">${this.parser.parseInline(tokens)}</a></h${depth}>\n`;
    },
  },
});

const read = (slug: Slug) => readFileSync(path.join(DIR, `${slug}.md`), "utf8");

export function renderDoc(slug: Slug) {
  const src = read(slug);
  const toc = marked
    .lexer(src)
    .filter((t): t is Token & { type: "heading"; depth: number; text: string } => t.type === "heading" && t.depth === 2)
    .map((t) => ({ id: slugify(t.text), text: t.text.replace(/[`*_]/g, "") }));
  // Links are written `concepts.md#x` so they also work on GitHub; the site serves `/docs/concepts#x`.
  const html = marked.parse(src, { async: false }).replace(/href="([\w-]+)\.md(#[^"]*)?"/g, 'href="$1$2"');
  return { html, toc };
}

export type SearchEntry = { slug: Slug; anchor: string; heading: string; text: string };

/** One entry per h1/h2/h3 section, plain text. ponytail: shipped as props (~tens of kB); move to a static JSON file if docs grow. */
export function searchIndex(): SearchEntry[] {
  const out: SearchEntry[] = [];
  for (const slug of SLUGS) {
    let cur: SearchEntry | undefined;
    for (const t of marked.lexer(read(slug))) {
      if (t.type === "heading" && t.depth <= 3) {
        cur = { slug, anchor: t.depth === 1 ? "" : slugify(t.text), heading: t.text.replace(/[`*_]/g, ""), text: "" };
        out.push(cur);
      } else if (cur && t.type !== "html") {
        cur.text += " " + t.raw.replace(/```\w*|[`*_#>|]|\[([^\]]*)\]\([^)]*\)/g, "$1").replace(/\s+/g, " ");
      }
    }
  }
  for (const e of out) e.text = e.text.trim().slice(0, 1500);
  return out;
}
