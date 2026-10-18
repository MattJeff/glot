import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { LocaleSwitcher, Logo, ThemeToggle } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { pageMetadata } from "@/lib/seo";
import { GITHUB_URL } from "@/lib/snippets";
import { PricingCards } from "./pricing-cards";

export async function generateMetadata({ params }: PageProps<"/[locale]/pricing">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "pricing" });
  return pageMetadata({ locale, path: "/pricing", ns: "pricing", title: t("meta.title"), description: t("meta.description") });
}

export default async function Pricing({ params }: PageProps<"/[locale]/pricing">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("pricing");
  const faq = ["oss", "byok", "selfHost", "limits", "tax"] as const;

  return (
    <div className="flex min-h-screen flex-col">
      <header className="border-b">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-6 px-4">
          <Link href="/"><Logo /></Link>
          <div className="ms-auto flex items-center gap-1">
            <LocaleSwitcher />
            <ThemeToggle />
            <Button size="sm" asChild>
              <Link href="/signup">{t("cta.free")}</Link>
            </Button>
          </div>
        </div>
      </header>
      <main id="main" className="mx-auto w-full max-w-6xl flex-1 px-4 py-12">
        <div className="mx-auto max-w-2xl text-center">
          <h1 className="text-3xl font-semibold tracking-tight sm:text-4xl">{t("title")}</h1>
          <p className="mt-3 text-muted-foreground">{t("subtitle")}</p>
        </div>

        <section className="mx-auto mt-10 max-w-3xl rounded-xl border bg-muted/30 p-5">
          <h2 className="font-semibold">{t("oss.title")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{t("oss.body")}</p>
          <Button variant="outline" size="sm" className="mt-3" asChild>
            <a href={GITHUB_URL} target="_blank" rel="noreferrer">{t("oss.cta")}</a>
          </Button>
        </section>

        <section className="mt-12" aria-labelledby="cloud">
          <h2 id="cloud" className="mb-2 text-center text-xl font-semibold">{t("cloud.title")}</h2>
          <p className="mx-auto mb-6 max-w-2xl text-center text-sm text-muted-foreground">{t("cloud.body")}</p>
          <PricingCards />
        </section>

        <section className="mx-auto mt-16 max-w-3xl" aria-labelledby="faq">
          <h2 id="faq" className="mb-4 text-xl font-semibold">{t("faq.title")}</h2>
          <dl className="space-y-5">
            {faq.map((q) => (
              <div key={q}>
                <dt className="font-medium">{t(`faq.${q}.q`)}</dt>
                <dd className="mt-1 text-sm text-muted-foreground">{t(`faq.${q}.a`)}</dd>
              </div>
            ))}
          </dl>
          <p className="mt-8 text-sm text-muted-foreground">
            {t.rich("compare", { link: (c) => <Link href="/#compare" className="underline underline-offset-2">{c}</Link> })}
          </p>
        </section>
      </main>
    </div>
  );
}
