import { ExternalLink, TriangleAlert } from "lucide-react";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { CodeBlock } from "@/components/common";
import { APP_URL, getRepoStats, SiteFooter, SiteHeader } from "@/components/landing";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import { type Locale, routing } from "@/i18n/routing";
import { pageMetadata, pageUrl, SITE } from "@/lib/seo";
import { EDGE_DNS, UNIVERSAL_TAG } from "@/lib/snippets";
import { chrome, ld, PLATFORMS, platform } from "../data";

export const dynamicParams = false;

export function generateStaticParams() {
  return PLATFORMS.map((p) => ({ id: p.id }));
}

export async function generateMetadata({ params }: PageProps<"/[locale]/integrations/[id]">): Promise<Metadata> {
  const { locale, id } = await params;
  const p = platform(id);
  if (!p) return {};
  const t = await getTranslations({ locale: locale as Locale, namespace: "integrations.page" });
  return pageMetadata({
    locale,
    path: `/integrations/${id}`,
    ns: "integrations",
    title: t("metaTitle", { name: p.name }),
    description: t("metaDescription", { name: p.name, method: p.method }),
    absolute: true, // the title already starts with "glot"
  });
}

function Steps({ steps, start = 1 }: { steps: string[]; start?: number }) {
  return (
    <ol start={start} className="list-decimal space-y-2 ps-5 marker:text-muted-foreground">
      {steps.map((s) => (
        <li key={s} className="ps-1 text-pretty">{s}</li>
      ))}
    </ol>
  );
}

export default async function PlatformPage({ params }: PageProps<"/[locale]/integrations/[id]">) {
  const { locale, id } = await params;
  const p = platform(id);
  if (!p) notFound();
  setRequestLocale(locale as Locale);
  const t = await getTranslations("integrations");
  const { stars } = await getRepoStats();
  const c = await chrome(stars);
  const name = p.name;
  const tag = p.injection.available;
  const prep = [t("page.prep.project"), t("page.prep.mode"), t("page.prep.copy")];
  const related = PLATFORMS.filter((x) => x.category === p.category && x.id !== p.id).slice(0, 6);
  const faq = [
    { q: t("page.faq.partner.q", { name }), a: t("page.faq.partner.a", { name }) },
    { q: t("page.faq.content.q", { name }), a: t("page.faq.content.a", { name, method: p.method }) },
    { q: t("page.faq.seo.q"), a: t("page.faq.seo.a") },
    ...p.faq,
  ];
  const howSteps = p.native ? p.native.steps : tag ? [...prep, ...p.injection.steps] : [];
  const url = pageUrl(locale, `/integrations/${id}`);
  const schema = [
    {
      "@context": "https://schema.org",
      "@type": "BreadcrumbList",
      itemListElement: [
        { "@type": "ListItem", position: 1, name: "glot", item: `${SITE}/` },
        { "@type": "ListItem", position: 2, name: t("title"), item: pageUrl(locale, "/integrations") },
        { "@type": "ListItem", position: 3, name: t("page.title", { name }), item: url },
      ],
    },
    ...(howSteps.length
      ? [
          {
            "@context": "https://schema.org",
            "@type": "HowTo",
            name: t("page.title", { name }),
            description: t("page.metaDescription", { name, method: p.method }),
            url,
            inLanguage: locale,
            step: howSteps.map((s, i) => ({ "@type": "HowToStep", position: i + 1, text: s })),
          },
        ]
      : []),
  ];

  return (
    <div className="flex min-h-screen flex-col">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: ld(schema) }} />
      <SiteHeader c={c} links={[]} stars={stars} />
      <main id="main" className="mx-auto w-full max-w-3xl flex-1 px-4 py-12">
        <nav aria-label={t("page.breadcrumb")} className="text-sm text-muted-foreground">
          <Link href="/integrations" className="underline-offset-2 hover:text-foreground hover:underline">
            {t("title")}
          </Link>
          <span aria-hidden className="mx-1.5">/</span>
          <span aria-current="page">{name}</span>
        </nav>
        <h1 className="mt-4 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">{t("page.title", { name })}</h1>
        <p className="mt-2 text-sm text-muted-foreground">
          {t(`categories.${p.category}`)} · {t(`methods.${p.method}`)}
        </p>
        <p className="mt-4 text-pretty text-lg text-muted-foreground">
          {tag || p.native ? t(`page.intro.${p.method}`, { name }) : t("page.unavailable", { name })}
        </p>

        {!p.verified && (
          <p role="note" className="mt-6 flex gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-sm">
            <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0 text-warning-foreground" />
            {t("page.unverified", { name })}
          </p>
        )}
        {locale !== routing.defaultLocale && <p className="mt-6 text-sm text-muted-foreground">{t("page.englishNote", { name })}</p>}

        {p.native && (
          <section className="mt-10" aria-labelledby="native">
            <h2 id="native" className="mb-4 text-xl font-semibold">{t(p.method === "plugin" ? "page.nativeTitle.plugin" : "page.nativeTitle.connector", { name })}</h2>
            <div lang="en" dir="ltr"><Steps steps={p.native.steps} /></div>
            <Link href={p.native.doc} className="mt-4 inline-block text-sm font-medium underline underline-offset-2">
              {t("page.nativeDoc")}
            </Link>
          </section>
        )}

        {tag && (
          <section className="mt-10" aria-labelledby="tag">
            <h2 id="tag" className="mb-1 text-xl font-semibold">{t(p.native ? "page.tagAltTitle" : "page.tagTitle")}</h2>
            {p.native && <p className="mb-4 text-sm text-muted-foreground">{t("page.tagAltBody", { name })}</p>}
            <div className="mt-4 space-y-5">
              <Steps steps={prep} />
              <CodeBlock code={UNIVERSAL_TAG} label={t("page.snippetLabel")} />
              <div lang="en" dir="ltr"><Steps steps={p.injection.steps} start={prep.length + 1} /></div>
            </div>
            <Link href="/docs/website" className="mt-4 inline-block text-sm font-medium underline underline-offset-2">
              {t("page.tagDoc")}
            </Link>
          </section>
        )}

        <section className="mt-10" aria-labelledby="plans">
          <h2 id="plans" className="mb-2 text-xl font-semibold">{t("page.plansTitle")}</h2>
          <p lang="en" dir="ltr" className="text-pretty">{p.injection.plans}</p>
          <p className="mt-2 text-sm text-muted-foreground">{t("page.glotPlan")}</p>
        </section>

        <section className="mt-10" aria-labelledby="seo">
          <h2 id="seo" className="mb-2 text-xl font-semibold">{t("page.seoTitle")}</h2>
          <p className="text-pretty text-muted-foreground">{p.seo.subdomain ? t("page.seoYes") : t("page.seoNo")}</p>
          {p.seo.subdomain && <CodeBlock code={EDGE_DNS} label={t("page.dnsLabel")} className="mt-4" />}
          <p lang="en" dir="ltr" className="mt-4 text-pretty">{p.seo.how}</p>
          <Link href="/docs/edge" className="mt-3 inline-block text-sm font-medium underline underline-offset-2">
            {t("page.edgeDoc")}
          </Link>
        </section>

        <section className="mt-10" aria-labelledby="faq">
          <h2 id="faq" className="mb-4 text-xl font-semibold">{t("page.faqTitle")}</h2>
          <dl className="space-y-5">
            {faq.map((f, i) => (
              <div key={f.q} lang={i < 3 ? undefined : "en"} dir={i < 3 ? undefined : "ltr"}>
                <dt className="font-medium">{f.q}</dt>
                <dd className="mt-1 text-sm text-pretty text-muted-foreground">{f.a}</dd>
              </div>
            ))}
          </dl>
        </section>

        {p.docs.length > 0 && (
          <section className="mt-10" aria-labelledby="docs">
            <h2 id="docs" className="mb-3 text-xl font-semibold">{t("page.docsTitle", { name })}</h2>
            <ul className="space-y-1.5 text-sm">
              {p.docs.map((d) => (
                <li key={d.url}>
                  <a href={d.url} target="_blank" rel="noreferrer" lang="en" dir="ltr" className="inline-flex items-center gap-1 underline underline-offset-2">
                    {d.label} <ExternalLink aria-hidden className="size-3.5" />
                  </a>
                </li>
              ))}
            </ul>
          </section>
        )}

        <section className="mt-12 rounded-xl border bg-muted/30 p-6">
          <h2 className="text-lg font-semibold">{t("page.cta.title", { name })}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("page.cta.body")}</p>
          <div className="mt-4 flex flex-wrap gap-2">
            <Button asChild>
              <a href={`${APP_URL}/signup`}>{t("page.cta.start")}</a>
            </Button>
            <Button variant="outline" asChild>
              <Link href="/docs">{t("page.cta.docs")}</Link>
            </Button>
          </div>
        </section>

        {related.length > 0 && (
          <section className="mt-10" aria-labelledby="related">
            <h2 id="related" className="mb-3 font-semibold">{t("page.related", { category: t(`categories.${p.category}`) })}</h2>
            <ul className="flex flex-wrap gap-2">
              {related.map((r) => (
                <li key={r.id}>
                  <Link href={`/integrations/${r.id}`} className="inline-block rounded-full border px-3 py-1 text-sm hover:bg-muted">
                    {r.name}
                  </Link>
                </li>
              ))}
            </ul>
          </section>
        )}

        <p className="mt-10 text-xs text-muted-foreground">{t("page.trademark", { name })}</p>
      </main>
      <SiteFooter c={c} links={[{ href: "/docs", label: c.docs }, { href: "/pricing", label: c.pricing }]} />
    </div>
  );
}
