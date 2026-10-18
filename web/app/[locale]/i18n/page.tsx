import { ArrowRight, Check, Minus, Package, Scale, Star, X } from "lucide-react";
import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Logo } from "@/components/common";
import {
  AgentsGrid,
  type Chrome,
  APP_URL,
  BenchChart,
  Faq,
  FitDemo,
  getRepoStats,
  GuaranteeGrid,
  HowSteps,
  LOCALIZE,
  NPM_URL,
  OverlayVisual,
  SectionHead,
  Showcase,
  SiteFooter,
  SiteHeader,
} from "@/components/landing";
import { CopyCommand, DemoVideo, Playground } from "@/components/landing-client";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { pageMetadata } from "@/lib/seo";
import { GITHUB_URL } from "@/lib/snippets";
import { cn } from "@/lib/utils";
import "../landing.css";

// Every cell is sourced (URL + date) in this file.
const COMPARE_SOURCES = `${GITHUB_URL}/blob/main/docs/comparison-sources.md`;

export async function generateMetadata({ params }: PageProps<"/[locale]/i18n">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "landing.meta" });
  return pageMetadata({ locale, path: "/i18n", ns: "landing", title: t("title"), description: t("description") });
}

export default async function I18nLanding({ params }: PageProps<"/[locale]/i18n">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("landing");
  const { stars, version } = await getRepoStats();

  type Cell = "yes" | "no" | "partial" | string;
  const vendors = ["glot", "lokalise", "crowdin", "lingo", "tolgee"] as const;
  // ponytail: checked 2026-10-18, see docs/comparison-sources.md before changing a cell.
  const rows: { label: string; cells: Record<(typeof vendors)[number], Cell> }[] = [
    { label: t("compare.rows.price"), cells: { glot: t("compare.glotPrice"), lokalise: t("compare.lokalisePrice"), crowdin: t("compare.crowdinPrice"), lingo: t("compare.lingoPrice"), tolgee: t("compare.tolgeePrice") } },
    { label: t("compare.rows.openSource"), cells: { glot: "yes", lokalise: "no", crowdin: "no", lingo: "partial", tolgee: "partial" } },
    { label: t("compare.rows.byok"), cells: { glot: "yes", lokalise: "no", crowdin: "yes", lingo: "yes", tolgee: "yes" } },
    { label: t("compare.rows.validator"), cells: { glot: "yes", lokalise: "partial", crowdin: "yes", lingo: "no", tolgee: "partial" } },
    { label: t("compare.rows.gitNative"), cells: { glot: "yes", lokalise: "partial", crowdin: "partial", lingo: "yes", tolgee: "partial" } },
    { label: t("compare.rows.mcp"), cells: { glot: "yes", lokalise: "yes", crowdin: "yes", lingo: "yes", tolgee: "yes" } },
    { label: t("compare.rows.inContext"), cells: { glot: "yes", lokalise: "yes", crowdin: "yes", lingo: "no", tolgee: "yes" } },
    { label: t("compare.rows.fit"), cells: { glot: "yes", lokalise: "partial", crowdin: "partial", lingo: "no", tolgee: "partial" } },
  ];
  const cell = (v: Cell) => {
    if (v === "yes") return <Check role="img" aria-label={t("compare.yes")} className="mx-auto size-4 text-success-foreground" />;
    if (v === "no") return <X role="img" aria-label={t("compare.no")} className="mx-auto size-4 text-muted-foreground" />;
    if (v === "partial") return <Minus role="img" aria-label={t("compare.partial")} className="mx-auto size-4 text-warning-foreground" />;
    return <span className="text-xs">{v}</span>;
  };

  const chrome: Chrome = {
    skip: t("skip"),
    home: t("nav.home"),
    nav: t("nav.label"),
    docs: t("nav.docs"),
    pricing: t("nav.pricing"),
    openApp: t("nav.openApp"),
    stars: t("nav.githubStars", { count: stars ?? 0 }),
    tagline: t("footer.tagline"),
    npm: t("footer.npm"),
    license: t("footer.license"),
  };
  const navLinks = [
    { href: "/", label: t("nav.experiments"), route: true },
    { href: "#how", label: t("nav.how") },
    { href: "#guarantees", label: t("nav.guarantees") },
    { href: "#try", label: t("nav.try") },
    { href: "#compare", label: t("nav.compare") },
  ];

  return (
    <div className="flex min-h-screen flex-col">
      <SiteHeader c={chrome} links={navLinks} stars={stars} />

      <main id="main" className="flex-1">
        {/* Hero: thesis, one command, proof. */}
        <section aria-labelledby="hero-title" className="relative overflow-hidden border-b">
          <div aria-hidden className="hero-glow pointer-events-none absolute inset-0" />
          <div aria-hidden className="bg-grid pointer-events-none absolute inset-0 [mask-image:radial-gradient(ellipse_at_top,black,transparent_65%)] opacity-50" />
          <div className="relative mx-auto max-w-6xl px-4 pt-16 pb-16 sm:pt-24 sm:pb-20">
            <div className="mx-auto max-w-3xl text-center">
              <p className="mb-6 inline-flex items-center gap-2 rounded-full border bg-background/80 px-3 py-1 text-xs text-muted-foreground">
                <span aria-hidden className="size-1.5 rounded-full bg-brand" />
                {t("hero.badge")}
              </p>
              <h1 id="hero-title" className="text-4xl font-semibold tracking-tighter text-balance sm:text-6xl">
                {t("hero.title")}
              </h1>
              <p className="mx-auto mt-5 max-w-2xl text-base text-pretty text-muted-foreground sm:text-lg">{t("hero.subtitle")}</p>
              <div className="mt-8 flex flex-col items-center justify-center gap-3 sm:flex-row">
                <CopyCommand command={LOCALIZE} className="max-w-full" />
                <Button size="lg" variant="outline" asChild className="h-11 px-5">
                  <a href={APP_URL}>
                    {t("hero.openApp")} <ArrowRight aria-hidden className="rtl:rotate-180" />
                  </a>
                </Button>
              </div>
              <p className="mt-3 text-xs text-muted-foreground">{t("hero.commandHint")}</p>
              <ul aria-label={t("hero.proofLabel")} className="mt-6 flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-sm text-muted-foreground">
                <li>
                  <a href={`${GITHUB_URL}/blob/main/LICENSE`} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1.5 hover:text-foreground">
                    <Scale aria-hidden className="size-4" /> {t("hero.proofOss")}
                  </a>
                </li>
                <li>
                  <a href={GITHUB_URL} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1.5 hover:text-foreground">
                    <Star aria-hidden className="size-4" /> <span className="tabular-nums">{t("hero.proofStars", { count: stars ?? 0 })}</span>
                  </a>
                </li>
                {version && (
                  <li>
                    <a href={NPM_URL} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1.5 hover:text-foreground">
                      <Package aria-hidden className="size-4" /> <span className="tabular-nums">{t("hero.proofNpm", { version })}</span>
                    </a>
                  </li>
                )}
              </ul>
            </div>

            <div className="mt-14 grid gap-4 lg:grid-cols-[1.6fr_1fr]">
              <figure className="hero-in min-w-0" style={{ animationDelay: "120ms" }}>
                <DemoVideo label={t("hero.demoAlt")} />
                <figcaption className="mt-2 text-xs text-muted-foreground">{t("hero.demoCaption")}</figcaption>
              </figure>
              <div className="hero-in min-w-0" style={{ animationDelay: "200ms" }}>
                <BenchChart />
              </div>
            </div>
          </div>
        </section>

        <section id="how" aria-labelledby="how-title" className="mx-auto max-w-6xl scroll-mt-16 px-4 py-24">
          <SectionHead id="how-title" eyebrow={t("how.eyebrow")} title={t("how.title")} subtitle={t("how.subtitle")} />
          <HowSteps />
        </section>

        <section id="guarantees" aria-labelledby="guarantees-title" className="scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="guarantees-title" eyebrow={t("guarantees.eyebrow")} title={t("guarantees.title")} subtitle={t("guarantees.subtitle")} />
            <GuaranteeGrid />
          </div>
        </section>

        <section aria-labelledby="fit-title" className="mx-auto max-w-6xl px-4 py-24">
          <Showcase id="fit-title" eyebrow={t("fit.eyebrow")} title={t("fit.title")} body={t("fit.body")} points={[t("fit.p1"), t("fit.p2"), t("fit.p3")]}>
            <FitDemo />
          </Showcase>
        </section>

        <section id="try" aria-labelledby="try-title" className="scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="try-title" eyebrow={t("play.eyebrow")} title={t("play.title")} subtitle={t("play.subtitle")} />
            <div className="mt-10">
              <Playground />
            </div>
          </div>
        </section>

        <section aria-labelledby="agents-title" className="mx-auto max-w-6xl px-4 py-24">
          <SectionHead id="agents-title" eyebrow={t("agents.eyebrow")} title={t("agents.title")} subtitle={t("agents.subtitle")} />
          <AgentsGrid />
        </section>

        <section aria-labelledby="overlay-title" className="border-t">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <Showcase
              flip
              id="overlay-title"
              eyebrow={t("overlay.eyebrow")}
              title={t("overlay.title")}
              body={t("overlay.body")}
              points={[t("overlay.p1"), t("overlay.p2"), t("overlay.p3")]}
            >
              <OverlayVisual />
            </Showcase>
          </div>
        </section>

        <section id="compare" aria-labelledby="compare-title" className="scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="compare-title" title={t("compare.title")} subtitle={t("compare.subtitle")} />
            <div className="reveal mt-10 overflow-x-auto rounded-xl border bg-background" tabIndex={0} aria-labelledby="compare-title">
              <table className="w-full min-w-[840px] text-sm">
                <thead>
                  <tr className="border-b text-muted-foreground">
                    <th scope="col" className="px-4 py-3 text-start font-medium">{t("compare.feature")}</th>
                    <th scope="col" className="bg-brand/5 px-4 py-3 text-center font-semibold text-foreground">
                      <Logo className="justify-center" />
                    </th>
                    {/* Vendor names are proper nouns. */}
                    <th scope="col" className="px-4 py-3 text-center font-medium">{"Lokalise"}</th>
                    <th scope="col" className="px-4 py-3 text-center font-medium">{"Crowdin"}</th>
                    <th scope="col" className="px-4 py-3 text-center font-medium">{"Lingo.dev"}</th>
                    <th scope="col" className="px-4 py-3 text-center font-medium">{"Tolgee"}</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((r) => (
                    <tr key={r.label} className="border-b transition-colors last:border-0 hover:bg-muted/40">
                      <th scope="row" className="px-4 py-3 text-start font-normal">{r.label}</th>
                      {vendors.map((v) => (
                        <td key={v} className={cn("px-4 py-3 text-center", v === "glot" ? "bg-brand/5 font-medium" : "text-muted-foreground")}>
                          {cell(r.cells[v])}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mt-3 text-xs text-muted-foreground">{t("compare.footnote")}</p>
            <p className="mt-1 text-xs">
              <a href={COMPARE_SOURCES} target="_blank" rel="noreferrer" className="text-muted-foreground underline underline-offset-2 hover:text-foreground">
                {t("compare.sources")}
              </a>
            </p>
          </div>
        </section>

        <section aria-labelledby="faq-title" className="mx-auto grid max-w-6xl gap-10 px-4 py-24 lg:grid-cols-[1fr_2fr]">
          <div className="reveal">
            <h2 id="faq-title" className="text-3xl font-semibold tracking-tight sm:text-4xl">{t("faq.title")}</h2>
            <Link href="/docs/faq" className="mt-3 inline-flex items-center gap-1 text-sm text-muted-foreground underline underline-offset-2 hover:text-foreground">
              {t("faq.more")}
            </Link>
          </div>
          <Faq />
        </section>

        <section aria-labelledby="cta-title" className="relative overflow-hidden border-t">
          <div aria-hidden className="hero-glow pointer-events-none absolute inset-0 rotate-180" />
          <div className="reveal relative mx-auto flex max-w-3xl flex-col items-center gap-5 px-4 py-24 text-center">
            <h2 id="cta-title" className="text-3xl font-semibold tracking-tight text-balance sm:text-5xl">{t("cta.title")}</h2>
            <p className="text-muted-foreground">{t("cta.body")}</p>
            <div className="flex flex-col items-center gap-3 sm:flex-row">
              <CopyCommand command={LOCALIZE} className="max-w-full" />
              <Button size="lg" variant="outline" asChild className="h-11 px-5">
                <Link href="/docs/getting-started">{t("cta.docs")}</Link>
              </Button>
            </div>
          </div>
        </section>
      </main>

      <SiteFooter
        c={chrome}
        links={[
          { href: "/", label: t("nav.experiments") },
          { href: "/docs", label: t("footer.docs") },
          { href: "/docs/benchmark", label: t("footer.benchmark") },
        ]}
      />
    </div>
  );
}
