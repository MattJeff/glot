import { statSync } from "node:fs";
import path from "node:path";
import type { MetadataRoute } from "next";
import { SLUGS } from "@/app/[locale]/docs/content";
import { languages, pageUrl, translatedLocales } from "@/lib/seo";

// Marketing pages: one entry per translated locale, with reciprocal hreflang. Docs are English only (canonical en).
const PAGES = [
  { path: "", ns: "home", priority: 1 },
  { path: "/i18n", ns: "landing", priority: 0.9 },
  { path: "/pricing", ns: "pricing", priority: 0.7 },
];

export default function sitemap(): MetadataRoute.Sitemap {
  const now = new Date();
  const pages = PAGES.flatMap((p) =>
    translatedLocales(p.ns).map((l) => ({
      url: pageUrl(l, p.path),
      lastModified: now,
      priority: p.priority,
      alternates: { languages: languages(p.ns, p.path) },
    })),
  );
  const mtime = (slug: string) => statSync(path.join(process.cwd(), "content", "docs", `${slug}.md`)).mtime;
  const docs = [
    { url: pageUrl("en", "/docs"), lastModified: now, priority: 0.6 },
    ...SLUGS.map((s) => ({ url: pageUrl("en", `/docs/${s}`), lastModified: mtime(s), priority: 0.5 })),
  ];
  return [...pages, ...docs];
}
