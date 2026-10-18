import { ArrowLeft, ArrowRight, Pencil } from "lucide-react";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { pageUrl } from "@/lib/seo";
import { editUrl, NAV, renderDoc, SLUGS, type Slug } from "../content";
import { CodeCopy, Toc } from "../docs-ui";

export const dynamicParams = false;

export function generateStaticParams() {
  return SLUGS.map((slug) => ({ slug }));
}

export async function generateMetadata({ params }: PageProps<"/[locale]/docs/[slug]">): Promise<Metadata> {
  const { locale, slug } = await params;
  if (!SLUGS.includes(slug as Slug)) return {};
  const t = await getTranslations({ locale: locale as Locale, namespace: "docs" });
  const { description } = renderDoc(slug as Slug);
  // Docs bodies are English in every locale: the English URL is canonical, no hreflang to duplicates.
  const url = pageUrl("en", `/docs/${slug}`);
  const title = t(`pages.${slug as Slug}`);
  return {
    title,
    description,
    alternates: { canonical: url },
    openGraph: { type: "article", url, siteName: "glot", title: `${title} · glot`, description, locale: "en_US" },
  };
}

export default async function DocPage({ params }: PageProps<"/[locale]/docs/[slug]">) {
  const { locale, slug } = await params;
  if (!SLUGS.includes(slug as Slug)) notFound();
  setRequestLocale(locale as Locale);
  const t = await getTranslations("docs");
  const { html, toc } = renderDoc(slug as Slug);
  const i = SLUGS.indexOf(slug as Slug);
  const prev = SLUGS[i - 1];
  const next = SLUGS[i + 1];
  const section = NAV.find((s) => (s.pages as readonly string[]).includes(slug))!.section;

  const pager = (to: Slug, dir: "prev" | "next") => (
    <Link
      href={`/docs/${to}`}
      rel={dir}
      className={`group flex flex-col gap-1 rounded-xl border p-4 transition-colors hover:border-foreground/30 hover:bg-muted/40 ${dir === "next" ? "items-end text-end sm:col-start-2" : ""}`}
    >
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        {dir === "prev" && <ArrowLeft aria-hidden className="size-3.5 transition-transform group-hover:-translate-x-0.5 rtl:rotate-180 rtl:group-hover:translate-x-0.5" />}
        {t(dir)}
        {dir === "next" && <ArrowRight aria-hidden className="size-3.5 transition-transform group-hover:translate-x-0.5 rtl:rotate-180 rtl:group-hover:-translate-x-0.5" />}
      </span>
      <span className="font-medium">{t(`pages.${to}`)}</span>
    </Link>
  );

  return (
    <div className="grid gap-12 xl:grid-cols-[minmax(0,1fr)_13rem]">
      <div className="min-w-0">
        <p className="mb-3 text-sm font-medium text-brand">{t(`sections.${section}`)}</p>
        {/* Content is English Markdown: marked as such for screen readers and browser translation. */}
        <article lang="en" dir="ltr" className="docs-prose" dangerouslySetInnerHTML={{ __html: html }} />
        <CodeCopy copy={t("copy")} copied={t("copied")} />
        <nav aria-label={t("pager")} className="mt-14 grid max-w-3xl gap-3 sm:grid-cols-2">
          {prev && pager(prev, "prev")}
          {next && pager(next, "next")}
        </nav>
        <a
          href={editUrl(slug as Slug)}
          target="_blank"
          rel="noreferrer"
          className="mt-6 inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
        >
          <Pencil aria-hidden className="size-3.5" /> {t("edit")}
        </a>
      </div>
      {toc.length > 1 && (
        <div className="sticky top-22 hidden max-h-[calc(100vh-7rem)] overflow-y-auto xl:block">
          <Toc items={toc} label={t("onThisPage")} />
        </div>
      )}
    </div>
  );
}
