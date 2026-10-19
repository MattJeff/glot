import {
  ArrowRight,
  BadgeCheck,
  BookOpenText,
  Braces,
  ChartNoAxesColumn,
  Code2,
  Hash,
  Languages,
  type LucideIcon,
  Package,
  Ruler,
  Scale,
  ShieldX,
  SquareFunction,
  Star,
} from "lucide-react";
import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { HomeDemo } from "@/components/home-demo";
import { APP_URL, type Chrome, getRepoStats, NPM_URL, RefusalCard, SectionHead, SiteFooter, SiteHeader } from "@/components/landing";
import { CopyCommand } from "@/components/landing-client";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { ORG_LD, pageMetadata, pageUrl, SITE } from "@/lib/seo";
import { GITHUB_URL } from "@/lib/snippets";
import "./landing.css";

const INIT = "npx glot-i18n init";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "home.meta" });
  return pageMetadata({ locale, ns: "home", title: t("title"), description: t("description"), absolute: true });
}

// Real refusals (glot-core validator and crates/core/src/policy.rs), shown verbatim.
const GUARANTEES: { key: "placeholders" | "plurals" | "glossary" | "width" | "patterns" | "facts"; icon: LucideIcon; bad: string; msg: string }[] = [
  { key: "placeholders", icon: Braces, bad: `de  "Jetzt {Preis} sparen"`, msg: "Placeholder `{Preis}` does not exist in the source; remove it or fix its name (the source only has {price})." },
  { key: "plurals", icon: SquareFunction, bad: `pl  "{count, plural, one {…} other {…}}"`, msg: "`{count, plural, …}` is missing plural categories required by pl: few, many." },
  { key: "glossary", icon: BookOpenText, bad: `fr  "Ouvrez votre espace"`, msg: "Glossary: `Workspace` must be translated as `Espace de travail` in fr." },
  { key: "width", icon: Ruler, bad: `de  "Jetzt kostenlos und ohne Kreditkarte starten"`, msg: "Too wide for its box in the app: one line of 160px at Geist 14px/500; it renders 301px wide." },
  {
    key: "patterns",
    icon: ShieldX,
    bad: `en  "Today only: unlimited invoices"`,
    msg: `"Today only": urgency or scarcity with no source. Declare the real deadline or stock as a fact (e.g. "offer ends 31 Oct") or remove the wording.`,
  },
  { key: "facts", icon: Hash, bad: `en  "Join 10,000 teams who get paid faster"`, msg: `"10000" is in neither the official text nor the facts. Declare it as a fact or remove it.` },
];

// web/content/docs/experiments.md, "Install (3 lines)".
const INSTALL = [
  { file: "proxy.ts", code: `export const proxy = glotProxy({ source: "en", locales: ["fr", "de"], experiments: { server: "https://glot.example.com", project: "acme", key: "PUBLIC_KEY" } });` },
  { file: "i18n/request.ts", code: `return { locale, messages: withExperiments(messages, { header: (await headers()).get("x-glot-xp"), locale }) };` },
  { file: "signup-form.tsx", code: `track("signup"); // or track("purchase", { value: 49, currency: "EUR" })` },
];

const FAQ = ["what", "audiences", "ai", "facts", "speed", "cookies", "seo", "traffic", "winner", "cost", "formats", "agents"] as const;

function Bullets({ items }: { items: { title: string; body: string }[] }) {
  return (
    <ul className="grid gap-x-10 gap-y-6 sm:grid-cols-2">
      {items.map((i) => (
        <li key={i.title} className="reveal flex gap-3">
          <BadgeCheck aria-hidden className="mt-0.5 size-5 shrink-0 text-brand" />
          <p className="text-pretty text-muted-foreground">
            <strong className="font-semibold text-foreground">{i.title}</strong> {i.body}
          </p>
        </li>
      ))}
    </ul>
  );
}

export default async function Home({ params }: PageProps<"/[locale]">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("home");
  const { stars, version } = await getRepoStats();

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
    { href: "#demo", label: t("nav.demo") },
    { href: "#how", label: t("nav.how") },
    { href: "#guarantees", label: t("nav.guarantees") },
    { href: "#results", label: t("nav.results") },
    { href: "/i18n", label: t("nav.i18n"), route: true },
  ];

  const faq = FAQ.map((k) => ({ q: t(`faq.${k}.q`), a: t(`faq.${k}.a`) }));
  // Structured data describes only what is visible on this page.
  const ld = [
    ORG_LD,
    {
      "@context": "https://schema.org",
      "@type": "SoftwareApplication",
      name: "glot",
      alternateName: ["glotlang", "glot-i18n"],
      url: `${SITE}/`,
      applicationCategory: "DeveloperApplication",
      operatingSystem: "macOS, Linux, Windows",
      description: t("meta.description"),
      inLanguage: locale,
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
      license: "https://opensource.org/licenses/MIT",
      ...(version ? { softwareVersion: version } : {}),
      downloadUrl: NPM_URL,
      codeRepository: GITHUB_URL,
      sameAs: [GITHUB_URL, NPM_URL],
      publisher: { "@id": `${SITE}/#org` },
    },
    {
      "@context": "https://schema.org",
      "@type": "FAQPage",
      url: pageUrl(locale),
      mainEntity: faq.map((f) => ({ "@type": "Question", name: f.q, acceptedAnswer: { "@type": "Answer", text: f.a } })),
    },
  ];

  const cta = (
    <div className="flex flex-col items-center justify-center gap-3 sm:flex-row sm:items-start">
      <CopyCommand command={INIT} className="max-w-full" />
      <div className="flex flex-col items-center gap-1.5">
        <Button size="lg" asChild className="h-11 bg-brand px-5 text-brand-foreground hover:bg-brand/90">
          <a href={`${APP_URL}/signup`}>
            {t("hero.start")} <ArrowRight aria-hidden className="rtl:rotate-180" />
          </a>
        </Button>
        <span className="text-xs text-muted-foreground">{t("hero.startHint")}</span>
      </div>
    </div>
  );

  return (
    <div className="flex min-h-screen flex-col">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(ld).replace(/</g, "\\u003c") }} />
      <SiteHeader c={chrome} links={navLinks} stars={stars} />

      <main id="main" className="flex-1">
        {/* Hero: outcome, mechanism, two CTAs, sourced proof. */}
        <section aria-labelledby="hero-title" className="relative overflow-hidden">
          <div aria-hidden className="hero-glow pointer-events-none absolute inset-0" />
          <div aria-hidden className="bg-grid pointer-events-none absolute inset-0 [mask-image:radial-gradient(ellipse_at_top,black,transparent_65%)] opacity-50" />
          <div className="relative mx-auto max-w-3xl px-4 pt-16 pb-12 text-center sm:pt-24">
            <p className="mb-6 inline-flex items-center gap-2 rounded-full border bg-background/80 py-1 ps-1 pe-3 text-xs text-muted-foreground">
              <span className="rounded-full bg-brand/10 px-2 py-0.5 font-medium text-brand">{t("hero.labs")}</span>
              {t("hero.badge")}
            </p>
            <h1 id="hero-title" className="text-4xl font-semibold tracking-tighter text-balance sm:text-6xl">
              {t("hero.title")}
            </h1>
            {/* No entrance animation on the text: it is the LCP. */}
            <p className="mx-auto mt-5 max-w-2xl text-base text-pretty text-muted-foreground sm:text-lg">
              {t("hero.subtitle")}
            </p>
            <div className="mt-8">
              {cta}
            </div>
            <p className="mt-3 text-xs text-muted-foreground">{t("hero.commandHint")}</p>
            <ul aria-label={t("hero.proofLabel")} className="mt-6 flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-sm text-muted-foreground">
              <li>
                <Link href="/docs/benchmark" title={t("hero.proofBenchNote")} className="inline-flex min-h-6 items-center gap-1.5 hover:text-foreground">
                  <ChartNoAxesColumn aria-hidden className="size-4 text-brand" />
                  <span className="tabular-nums">{t("hero.proofBench")}</span>
                  <sup aria-hidden>1</sup>
                </Link>
              </li>
              <li>
                <a href={`${GITHUB_URL}/blob/main/LICENSE`} target="_blank" rel="noreferrer" className="inline-flex min-h-6 items-center gap-1.5 hover:text-foreground">
                  <Scale aria-hidden className="size-4" /> {t("hero.proofOss")}
                </a>
              </li>
              <li>
                <a href={GITHUB_URL} target="_blank" rel="noreferrer" className="inline-flex min-h-6 items-center gap-1.5 hover:text-foreground">
                  <Star aria-hidden className="size-4" /> <span className="tabular-nums">{t("hero.proofStars", { count: stars ?? 0 })}</span>
                </a>
              </li>
              {version && (
                <li>
                  <a href={NPM_URL} target="_blank" rel="noreferrer" className="inline-flex min-h-6 items-center gap-1.5 hover:text-foreground">
                    <Package aria-hidden className="size-4" /> <span className="tabular-nums">{t("hero.proofNpm", { version })}</span>
                  </a>
                </li>
              )}
            </ul>
            <p className="mt-2 text-[11px] text-muted-foreground">
              <sup aria-hidden>1 </sup>
              {t("hero.proofBenchNote")}
            </p>
          </div>
        </section>

        {/* The demo is the page's centrepiece. */}
        <section id="demo" aria-labelledby="demo-title" className="relative scroll-mt-16 border-b">
          <div className="mx-auto max-w-6xl px-4 pb-20">
            <SectionHead center id="demo-title" eyebrow={t("demo.eyebrow")} title={t("demo.title")} subtitle={t("demo.subtitle")} />
            <div className="mt-10">
              <HomeDemo />
            </div>
            <p className="mt-3 text-center text-xs text-muted-foreground">{t("demo.caption")}</p>
          </div>
        </section>

        <section aria-labelledby="problem-title" className="mx-auto max-w-6xl px-4 py-24">
          <SectionHead id="problem-title" eyebrow={t("problem.eyebrow")} title={t("problem.title")} />
          <ol className="mt-10 grid gap-4 md:grid-cols-3">
            {(["one", "ai", "manual"] as const).map((k, i) => (
              <li key={k} className="reveal rounded-xl border bg-card p-5 shadow-xs" style={{ animationDelay: `${i * 60}ms` }}>
                <p className="font-mono text-xs text-muted-foreground tabular-nums">{String(i + 1).padStart(2, "0")}</p>
                <h3 className="mt-2 font-medium">{t(`problem.${k}.title`)}</h3>
                <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{t(`problem.${k}.body`)}</p>
                {k === "ai" && (
                  <p className="mt-3 border-t pt-3 text-xs text-muted-foreground">
                    {t("problem.note")}{" "}
                    <Link href="/docs/benchmark" className="font-medium text-foreground underline underline-offset-2 hover:text-brand">
                      {t("problem.noteLink")}
                    </Link>
                  </p>
                )}
              </li>
            ))}
          </ol>
          <p className="reveal mt-8 text-lg font-medium text-pretty">{t("problem.kicker")}</p>
        </section>

        <section id="how" aria-labelledby="how-title" className="scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="how-title" eyebrow={t("how.eyebrow")} title={t("how.title")} />
            <ol className="mt-12 grid gap-4 md:grid-cols-3">
              {(["pick", "write", "ship"] as const).map((k, i) => (
                <li key={k} className="reveal relative rounded-xl border bg-card p-5 shadow-xs" style={{ animationDelay: `${i * 80}ms` }}>
                  <span className="grid size-7 place-items-center rounded-full border bg-background font-mono text-xs tabular-nums">{i + 1}</span>
                  <h3 className="mt-3 font-medium">{t(`how.${k}.title`)}</h3>
                  <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{t(`how.${k}.body`)}</p>
                  {i < 2 && <ArrowRight aria-hidden className="absolute top-6 -end-3.5 z-10 hidden size-5 rounded-full border bg-background p-0.5 text-muted-foreground md:block rtl:rotate-180" />}
                </li>
              ))}
            </ol>
            <p className="mt-6 text-sm text-muted-foreground">{t("how.footnote")}</p>
          </div>
        </section>

        <section id="guarantees" aria-labelledby="guarantees-title" className="mx-auto max-w-6xl scroll-mt-16 px-4 py-24">
          <SectionHead id="guarantees-title" eyebrow={t("guarantees.eyebrow")} title={t("guarantees.title")} subtitle={t("guarantees.subtitle")} />
          <ul className="mt-12 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {GUARANTEES.map(({ key, icon, bad, msg }, i) => (
              <RefusalCard key={key} id={`hg-${key}`} i={i} icon={icon} title={t(`guarantees.${key}.title`)} body={t(`guarantees.${key}.body`)} bad={bad} msg={msg} refusal={t("guarantees.refusal")} />
            ))}
          </ul>
          <p className="mt-6 text-xs text-muted-foreground">{t("guarantees.patternsNote")}</p>
        </section>

        <section id="results" aria-labelledby="results-title" className="scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="results-title" eyebrow={t("results.eyebrow")} title={t("results.title")} subtitle={t("results.subtitle")} />
            <figure className="reveal mt-10 rounded-xl border bg-card p-5 shadow-xs">
              <figcaption className="mb-2 inline-flex rounded-full border border-dashed px-2 py-0.5 text-[11px] font-medium text-muted-foreground">{t("results.example")}</figcaption>
              <blockquote className="text-lg font-medium text-pretty sm:text-xl">“{t("results.exampleSentence")}”</blockquote>
            </figure>
            <div className="mt-10">
              <Bullets items={(["noEarly", "noFake", "segment", "traffic", "alarms", "audiences"] as const).map((k) => ({ title: t(`results.${k}.title`), body: t(`results.${k}.body`) }))} />
            </div>
          </div>
        </section>

        <section aria-labelledby="goals-title" className="mx-auto max-w-6xl px-4 py-24">
          <SectionHead id="goals-title" eyebrow={t("goals.eyebrow")} title={t("goals.title")} subtitle={t("goals.subtitle")} />
          <dl className="mt-10 grid gap-4 md:grid-cols-3">
            {(["goal", "guardrail", "holdout"] as const).map((k) => (
              <div key={k} className="reveal rounded-xl border bg-card p-5 shadow-xs">
                <dt className="font-medium">{t(`goals.${k}.title`)}</dt>
                <dd className="mt-2 text-sm leading-relaxed text-muted-foreground">{t(`goals.${k}.body`)}</dd>
              </div>
            ))}
          </dl>
        </section>

        <section aria-labelledby="delivery-title" className="border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="delivery-title" eyebrow={t("delivery.eyebrow")} title={t("delivery.title")} />
            <div className="mt-10">
              <Bullets items={(["flicker", "path", "cookie", "seo"] as const).map((k) => ({ title: t(`delivery.${k}.title`), body: t(`delivery.${k}.body`) }))} />
            </div>
            <p className="mt-6 text-xs text-muted-foreground">{t("delivery.small")}</p>
          </div>
        </section>

        <section aria-labelledby="install-title" className="mx-auto grid max-w-6xl items-center gap-10 px-4 py-24 lg:grid-cols-[2fr_3fr]">
          <div>
            <SectionHead id="install-title" eyebrow={t("install.eyebrow")} title={t("install.title")} subtitle={t("install.subtitle")} />
            <Link href="/docs/experiments" className="mt-6 inline-flex min-h-6 items-center gap-1 text-sm font-medium underline underline-offset-2 hover:text-brand">
              {t("install.docs")} <ArrowRight aria-hidden className="size-3.5 rtl:rotate-180" />
            </Link>
          </div>
          <ol aria-label={t("install.label")} dir="ltr" className="reveal min-w-0 space-y-3">
            {INSTALL.map((s, i) => (
              <li key={s.file} className="overflow-hidden rounded-xl border term">
                <p className="term-rule term-faint flex items-center gap-2 border-b px-3 py-1.5 font-mono text-[11px]">
                  <Code2 aria-hidden className="size-3.5" /> <span className="tabular-nums">{i + 1}</span> {s.file}
                </p>
                <pre className="px-3 py-2.5 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap">
                  <code>{s.code}</code>
                </pre>
              </li>
            ))}
          </ol>
        </section>

        <section aria-labelledby="who-title" className="border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="who-title" eyebrow={t("who.eyebrow")} title={t("who.title")} />
            <ul className="mt-10 grid gap-4 md:grid-cols-3">
              {(["dev", "growth", "l10n"] as const).map((k) => (
                <li key={k} className="reveal rounded-xl border bg-card p-5 shadow-xs">
                  <h3 className="font-medium">{t(`who.${k}.title`)}</h3>
                  <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{t(`who.${k}.body`)}</p>
                </li>
              ))}
            </ul>
          </div>
        </section>

        <section aria-labelledby="integrations-title" className="mx-auto max-w-6xl px-4 py-24">
          <SectionHead id="integrations-title" eyebrow={t("integrations.eyebrow")} title={t("integrations.title")} />
          <dl className="reveal mt-10 divide-y rounded-xl border bg-card">
            {(["i18n", "frameworks", "edge", "agents", "git", "models"] as const).map((k) => (
              <div key={k} className="grid gap-1 px-5 py-4 sm:grid-cols-[10rem_1fr] sm:gap-6">
                <dt className="text-sm font-medium">{t(`integrations.${k}.label`)}</dt>
                <dd className="text-sm text-muted-foreground">{t(`integrations.${k}.value`)}</dd>
              </div>
            ))}
          </dl>
        </section>

        {/* The localization engine underneath, and the door to the former landing. */}
        <section aria-labelledby="foundation-title" className="relative overflow-hidden border-y bg-muted/30">
          <div className="mx-auto grid max-w-6xl items-center gap-10 px-4 py-24 lg:grid-cols-2">
            <div>
              <SectionHead id="foundation-title" eyebrow={t("foundation.eyebrow")} title={t("foundation.title")} subtitle={t("foundation.subtitle")} />
              <div className="mt-6 flex flex-wrap items-center gap-3">
                <Button asChild size="lg" className="h-11 px-5">
                  <Link href="/i18n">
                    <Languages aria-hidden /> {t("foundation.link")}
                  </Link>
                </Button>
              </div>
            </div>
            <div className="reveal space-y-4 rounded-xl border bg-card p-5 shadow-xs sm:p-6">
              <CopyCommand command={t("cta.localize_command")} className="max-w-full" />
              <p className="text-sm leading-relaxed">{t("foundation.proof")}</p>
              <p className="text-sm leading-relaxed text-muted-foreground">{t("foundation.cost")}</p>
              <Link href="/docs/benchmark" className="inline-flex min-h-6 items-center gap-1 text-sm font-medium underline underline-offset-2 hover:text-brand">
                {t("foundation.bench")} <ArrowRight aria-hidden className="size-3.5 rtl:rotate-180" />
              </Link>
            </div>
          </div>
        </section>

        <section aria-labelledby="faq-title" className="mx-auto grid max-w-6xl gap-10 px-4 py-24 lg:grid-cols-[1fr_2fr]">
          <div className="reveal">
            <h2 id="faq-title" className="text-3xl font-semibold tracking-tight sm:text-4xl">{t("faq.title")}</h2>
            <Link href="/docs/faq" className="mt-3 inline-flex min-h-6 items-center gap-1 text-sm text-muted-foreground underline underline-offset-2 hover:text-foreground">
              {t("faq.more")}
            </Link>
          </div>
          <div className="divide-y rounded-xl border bg-card">
            {faq.map((f) => (
              <details key={f.q} className="faq group">
                <summary className="flex min-h-14 cursor-pointer list-none items-center justify-between gap-4 px-5 py-4 font-medium hover:bg-muted/40 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
                  {f.q}
                  <span aria-hidden className="grid size-6 shrink-0 place-items-center rounded-full border text-muted-foreground transition-transform duration-200 group-open:rotate-45">
                    +
                  </span>
                </summary>
                <p className="px-5 pb-5 text-sm leading-relaxed text-muted-foreground">{f.a}</p>
              </details>
            ))}
          </div>
        </section>

        <section aria-labelledby="pricing-title" className="mx-auto max-w-6xl px-4 pb-24">
          <div className="reveal flex flex-col items-start gap-5 rounded-2xl border bg-card p-6 shadow-xs sm:p-8 md:flex-row md:items-center">
            <div className="flex-1">
              <h2 id="pricing-title" className="text-2xl font-semibold tracking-tight text-balance">{t("pricing.title")}</h2>
              <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted-foreground">{t("pricing.body")}</p>
            </div>
            <Button asChild variant="outline" size="lg" className="h-11 px-5">
              <Link href="/pricing">{t("pricing.cta")}</Link>
            </Button>
          </div>
        </section>

        <section aria-labelledby="cta-title" className="relative overflow-hidden border-t">
          <div aria-hidden className="hero-glow pointer-events-none absolute inset-0 rotate-180" />
          <div className="reveal relative mx-auto flex max-w-3xl flex-col items-center gap-5 px-4 py-24 text-center">
            <h2 id="cta-title" className="text-3xl font-semibold tracking-tight text-balance sm:text-5xl">{t("cta.title")}</h2>
            <p className="text-muted-foreground">{t("cta.body")}</p>
            {cta}
            <Link href="/docs/experiments" className="inline-flex min-h-6 items-center text-sm text-muted-foreground underline underline-offset-2 hover:text-foreground">
              {t("cta.docs")}
            </Link>
          </div>
        </section>
      </main>

      <SiteFooter
        c={chrome}
        links={[
          { href: "/i18n", label: t("footer.i18n") },
          { href: "/docs", label: t("footer.docs") },
          { href: "/docs/benchmark", label: t("footer.benchmark") },
          { href: "/pricing", label: t("footer.pricing") },
        ]}
      />
    </div>
  );
}
