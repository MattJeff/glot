import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { getRepoStats, SiteFooter, SiteHeader } from "@/components/landing";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { pageMetadata, pageUrl } from "@/lib/seo";
import { CATEGORIES, chrome, ld, PLATFORMS } from "./data";
import { IntegrationsGrid } from "./grid";

export async function generateMetadata({ params }: PageProps<"/[locale]/integrations">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "integrations" });
  return pageMetadata({
    locale,
    path: "/integrations",
    ns: "integrations",
    title: t("meta.title"),
    description: t("meta.description", { count: PLATFORMS.length }),
  });
}

export default async function Integrations({ params }: PageProps<"/[locale]/integrations">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("integrations");
  const { stars } = await getRepoStats();
  const c = await chrome(stars);
  const items = PLATFORMS.map(({ id, name, category, method }) => ({ id, name, category, method }));
  const list = {
    "@context": "https://schema.org",
    "@type": "ItemList",
    name: t("title"),
    url: pageUrl(locale, "/integrations"),
    itemListElement: PLATFORMS.map((p, i) => ({
      "@type": "ListItem",
      position: i + 1,
      name: t("page.title", { name: p.name }),
      url: pageUrl(locale, `/integrations/${p.id}`),
    })),
  };

  return (
    <div className="flex min-h-screen flex-col">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: ld(list) }} />
      <SiteHeader c={c} links={[]} stars={stars} />
      <main id="main" className="mx-auto w-full max-w-6xl flex-1 px-4 py-12">
        <div className="max-w-2xl">
          <h1 className="text-3xl font-semibold tracking-tight text-balance sm:text-4xl">{t("title")}</h1>
          <p className="mt-3 text-pretty text-muted-foreground">{t("subtitle")}</p>
        </div>
        <div className="mt-8">
          <IntegrationsGrid items={items} categories={CATEGORIES} />
        </div>
        <section className="mt-14 max-w-2xl rounded-xl border bg-muted/30 p-5">
          <h2 className="font-semibold">{t("generic.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("generic.body")}</p>
          <Link href="/docs/website" className="mt-3 inline-block text-sm font-medium underline underline-offset-2">
            {t("generic.cta")}
          </Link>
        </section>
      </main>
      <SiteFooter c={c} links={[{ href: "/docs", label: c.docs }, { href: "/pricing", label: c.pricing }]} />
    </div>
  );
}
