import { readFileSync } from "node:fs";
import path from "node:path";
import { marked } from "marked";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { setRequestLocale } from "next-intl/server";
import { chrome } from "@/app/[locale]/integrations/data";
import { getRepoStats, SiteFooter, SiteHeader } from "@/components/landing";
import type { Locale } from "@/i18n/routing";
import { SITE } from "@/lib/seo";

// Legal pages (KAIROS SAS). French is authoritative; every other locale reads the English text.
const PAGES = ["legal", "privacy", "terms"] as const;
type Page = (typeof PAGES)[number];
export const dynamicParams = false;
export function generateStaticParams() {
  return PAGES.map((page) => ({ page }));
}

function read(page: Page, locale: string) {
  const lang = locale === "fr" ? "fr" : "en";
  const src = readFileSync(path.join(process.cwd(), "content", "legal", lang, `${page}.md`), "utf8");
  return { title: src.match(/^# (.+)$/m)?.[1] ?? page, html: marked.parse(src, { async: false }) as string, lang };
}

export async function generateMetadata({ params }: PageProps<"/[locale]/legal/[page]">): Promise<Metadata> {
  const { locale, page } = await params;
  if (!PAGES.includes(page as Page)) return {};
  const { title } = read(page as Page, locale);
  const canonical = `${SITE}${locale === "fr" ? "/fr" : ""}/legal/${page}`;
  return { title, alternates: { canonical } };
}

export default async function LegalPage({ params }: PageProps<"/[locale]/legal/[page]">) {
  const { locale, page } = await params;
  if (!PAGES.includes(page as Page)) notFound();
  setRequestLocale(locale as Locale);
  const { stars } = await getRepoStats();
  const c = await chrome(stars);
  const { html, lang } = read(page as Page, locale);
  return (
    <div className="flex min-h-screen flex-col">
      <SiteHeader c={c} links={[]} stars={stars} />
      <main id="main" lang={lang} className="mx-auto w-full max-w-3xl flex-1 px-4 py-14">
        <article
          className="prose-legal space-y-4 text-[15px] leading-relaxed [&_a]:underline [&_h1]:text-3xl [&_h1]:font-bold [&_h1]:tracking-tight [&_h2]:mt-10 [&_h2]:text-xl [&_h2]:font-semibold [&_li]:ms-5 [&_li]:list-disc [&_table]:w-full [&_table]:text-sm [&_td]:border-t [&_td]:py-2 [&_td]:pe-3 [&_td]:align-top [&_th]:pe-3 [&_th]:pb-2 [&_th]:text-start"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </main>
      <SiteFooter c={c} links={[]} />
    </div>
  );
}
