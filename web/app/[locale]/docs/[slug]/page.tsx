import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { editUrl, renderDoc, SLUGS, type Slug } from "../content";

export const dynamicParams = false;

export function generateStaticParams() {
  return SLUGS.map((slug) => ({ slug }));
}

export async function generateMetadata({ params }: PageProps<"/[locale]/docs/[slug]">) {
  const { locale, slug } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "docs" });
  return { title: SLUGS.includes(slug as Slug) ? t(`pages.${slug as Slug}`) : t("title") };
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

  return (
    <div className="grid gap-10 xl:grid-cols-[1fr_12rem]">
      <div className="min-w-0">
        {/* Content is English Markdown: marked as such for screen readers and browser translation. */}
        <article lang="en" dir="ltr" className="docs-prose" dangerouslySetInnerHTML={{ __html: html }} />
        <div className="mt-12 flex flex-wrap items-center justify-between gap-4 border-t pt-6 text-sm">
          <a href={editUrl(slug as Slug)} target="_blank" rel="noreferrer" className="text-muted-foreground hover:text-foreground">
            {t("edit")}
          </a>
          <div className="flex gap-4">
            {prev && (
              <Link href={`/docs/${prev}`} className="hover:underline">
                ← {t(`pages.${prev}`)}
              </Link>
            )}
            {next && (
              <Link href={`/docs/${next}`} className="hover:underline">
                {t(`pages.${next}`)} →
              </Link>
            )}
          </div>
        </div>
      </div>
      {toc.length > 1 && (
        <nav aria-label={t("onThisPage")} className="sticky top-22 hidden max-h-[calc(100vh-7rem)] overflow-y-auto text-sm xl:block">
          <p className="mb-2 font-medium">{t("onThisPage")}</p>
          <ul lang="en" className="space-y-1">
            {toc.map((h) => (
              <li key={h.id}>
                <a href={`#${h.id}`} className="text-muted-foreground hover:text-foreground">
                  {h.text}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      )}
    </div>
  );
}
