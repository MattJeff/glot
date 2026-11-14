import {
  ArrowRight,
  BookOpenText,
  Bot,
  ChartNoAxesColumn,
  Check,
  Compass,
  Eye,
  EyeOff,
  FileClock,
  Globe,
  Link2,
  type LucideIcon,
  Mail,
  MapPin,
  MessagesSquare,
  MousePointerClick,
  RefreshCw,
  Scale,
  Search,
  ShieldCheck,
  ShoppingBag,
  Smartphone,
  Star,
  Trophy,
  Unlock,
  Wand2,
  X,
  Zap,
} from "lucide-react";
import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { HomeDemo } from "@/components/home-demo";
import { APP_URL, type Chrome, getRepoStats, NPM_URL, SectionHead, SiteFooter, SiteHeader } from "@/components/landing";
import { ACCENTS } from "@/components/landing-accents";
import { LangMorph, WinnerBoard } from "@/components/landing-visuals";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { ORG_LD, pageMetadata, pageUrl, SITE } from "@/lib/seo";
import { GITHUB_URL } from "@/lib/snippets";
import "./landing.css";

export async function generateMetadata({ params }: PageProps<"/[locale]">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "home.meta" });
  return pageMetadata({ locale, ns: "home", title: t("title"), description: t("description"), absolute: true });
}

// "Hello" in the languages glot ships; universal, so not in the catalogs.
const HELLOS = ["Hello", "Bonjour", "Hola", "Hallo", "Ciao", "Olá", "こんにちは", "안녕하세요", "你好", "Привет", "مرحبا", "שלום", "नमस्ते", "Merhaba", "Cześć", "Hej", "Xin chào", "Γειά σου", "Halo", "Sawubona", "สวัสดี", "Ahoj"];

function HelloMarquee() {
  const row = HELLOS.map((h, i) => (
    <span key={`${h}${i}`} className={`hello text-sm ${ACCENTS[i % ACCENTS.length]}`}>
      {h}
    </span>
  ));
  return (
    <div aria-hidden className="marquee py-6" dir="ltr">
      <div className="marquee-track">
        {row}
        {row}
      </div>
    </div>
  );
}

// Every feature, in plain words (the technical detail lives in /docs). Keys under home.features.
type Group = { id: "app" | "web" | "beyond" | "grow" | "trust"; anchor: string; acc: string; items: [string, LucideIcon][] };
const GROUPS: Record<Group["id"], Group> = {
  app: { id: "app", anchor: "app", acc: "acc-green", items: [["minutes", Zap], ["safe", ShieldCheck], ["fresh", RefreshCw], ["brand", BookOpenText], ["onpage", MousePointerClick]] },
  web: { id: "web", anchor: "website", acc: "acc-cyan", items: [["tag", Wand2], ["google", Search], ["local", MapPin], ["shop", ShoppingBag], ["live", Eye], ["longtail", Globe]] },
  beyond: { id: "beyond", anchor: "beyond", acc: "acc-violet", items: [["mobile", Smartphone], ["emails", Mail], ["ai", Bot], ["support", MessagesSquare], ["once", Link2]] },
  grow: { id: "grow", anchor: "grow", acc: "acc-amber", items: [["next", Compass], ["words", Trophy], ["private", EyeOff]] },
  trust: { id: "trust", anchor: "trust", acc: "acc-pink", items: [["proof", FileClock], ["open", Unlock]] },
};

const FAQ = ["dev", "quality", "tools", "google", "privacy", "price"] as const;

type T = Awaited<ReturnType<typeof getTranslations<"home">>>;

function FeatureGroup({ g, t, wash }: { g: Group; t: T; wash?: boolean }) {
  const k = (s: string) => `features.${g.id}.${s}` as Parameters<T>[0];
  return (
    <section id={g.anchor} aria-labelledby={`${g.anchor}-title`} className={`${g.acc} scroll-mt-16 ${wash ? "section-wash border-y bg-muted/30" : ""}`}>
      <div className="mx-auto max-w-6xl px-4 py-24">
        <SectionHead id={`${g.anchor}-title`} eyebrow={t(k("eyebrow"))} title={t(k("title"))} subtitle={t(k("subtitle"))} />
        <ul className={`mt-12 grid gap-4 sm:grid-cols-2 ${g.items.length === 2 ? "" : "lg:grid-cols-3"}`}>
          {g.items.map(([id, Icon], i) => (
            // ponytail: 5 cards fill a 3-column grid when the first one spans two.
            <li key={id} className={`reveal card-acc flex flex-col ${g.items.length === 5 && i === 0 ? "lg:col-span-2" : ""} rounded-xl border bg-card p-5 shadow-xs ${ACCENTS[(ACCENTS.indexOf(g.acc as (typeof ACCENTS)[number]) + i) % ACCENTS.length]}`} style={{ animationDelay: `${(i % 3) * 60}ms` }}>
              <span className="tile size-10">
                <Icon aria-hidden className="size-5" />
              </span>
              <h3 className="mt-4 font-semibold">{t(k(`${id}.title`))}</h3>
              <p className="mt-3 flex gap-2 text-sm leading-relaxed text-muted-foreground">
                <X aria-hidden className="mt-0.5 size-4 shrink-0 opacity-60" />
                <span>
                  <span className="sr-only">{t("features.before")}: </span>
                  {t(k(`${id}.pain`))}
                </span>
              </p>
              <p className="mt-2 flex gap-2 text-sm leading-relaxed">
                <Check aria-hidden className="acc-ink mt-0.5 size-4 shrink-0" />
                <span>
                  <span className="sr-only">{t("features.after")}: </span>
                  {t(k(`${id}.gain`))}
                </span>
              </p>
            </li>
          ))}
        </ul>
      </div>
    </section>
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
    { href: "#app", label: t("nav.app") },
    { href: "#website", label: t("nav.website") },
    { href: "#beyond", label: t("nav.beyond") },
    { href: "#demo", label: t("nav.demo") },
    { href: "#results", label: t("nav.results") },
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
      applicationCategory: "BusinessApplication",
      operatingSystem: "Web",
      description: t("meta.description"),
      inLanguage: locale,
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
      license: "https://opensource.org/licenses/MIT",
      ...(version ? { softwareVersion: version } : {}),
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
      <div className="flex flex-col items-center gap-1.5">
        <Button size="lg" asChild className="h-12 bg-brand px-6 text-base text-brand-foreground hover:bg-brand/90">
          <a href={`${APP_URL}/signup`}>
            {t("hero.start")} <ArrowRight aria-hidden className="rtl:rotate-180" />
          </a>
        </Button>
        <span className="text-xs text-muted-foreground">{t("hero.startHint")}</span>
      </div>
      <Button size="lg" variant="outline" asChild className="h-12 px-6 text-base">
        <a href="#demo">{t("hero.see")}</a>
      </Button>
    </div>
  );

  return (
    <div className="flex min-h-screen flex-col">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(ld).replace(/</g, "\\u003c") }} />
      <SiteHeader c={chrome} links={navLinks} stars={stars} />

      <main id="main" className="flex-1">
        <section aria-labelledby="hero-title" className="relative overflow-hidden">
          <div aria-hidden className="aurora pointer-events-none">
            <span />
            <span />
            <span />
            <span />
          </div>
          <div aria-hidden className="bg-grid pointer-events-none absolute inset-0 [mask-image:radial-gradient(ellipse_at_top,black,transparent_65%)] opacity-50" />
          <div className="relative mx-auto max-w-3xl px-4 pt-16 pb-10 text-center sm:pt-24">
            <p className="acc-violet tile mx-auto mb-6 w-fit rounded-full px-3 py-1 text-xs font-medium">{t("hero.badge")}</p>
            <h1 id="hero-title" className="text-4xl font-semibold tracking-tighter text-balance sm:text-6xl">
              {t("hero.title")}
            </h1>
            {/* No entrance animation on the text: it is the LCP. */}
            <p className="mx-auto mt-5 max-w-2xl text-base text-pretty text-muted-foreground sm:text-lg">{t("hero.subtitle")}</p>
            <div className="mt-8">{cta}</div>
            <ul aria-label={t("hero.proofLabel")} className="mt-8 flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-sm text-muted-foreground">
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
            </ul>
            <p className="mt-2 text-[11px] text-muted-foreground">
              <sup aria-hidden>1 </sup>
              {t("hero.proofBenchNote")}
            </p>
          </div>
        </section>

        <HelloMarquee />

        <section aria-labelledby="problem-title" className="acc-coral mx-auto max-w-6xl px-4 py-24">
          <SectionHead id="problem-title" eyebrow={t("problem.eyebrow")} title={t("problem.title")} />
          <ol className="mt-10 grid gap-4 md:grid-cols-3">
            {(["one", "ai", "manual"] as const).map((k, i) => (
              <li key={k} className={`reveal card-acc rounded-xl border bg-card p-5 shadow-xs ${["acc-coral", "acc-amber", "acc-pink"][i]}`} style={{ animationDelay: `${i * 60}ms` }}>
                <p className="acc-ink font-mono text-3xl font-semibold tracking-tight tabular-nums">{String(i + 1).padStart(2, "0")}</p>
                <h3 className="mt-2 font-semibold">{t(`problem.${k}.title`)}</h3>
                <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{t(`problem.${k}.body`)}</p>
              </li>
            ))}
          </ol>
          <p className="reveal mt-8 text-lg font-medium text-pretty">{t("problem.kicker")}</p>
        </section>

        <section id="how" aria-labelledby="how-title" className="acc-cyan section-wash scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="how-title" eyebrow={t("how.eyebrow")} title={t("how.title")} />
            <div className="mt-12 grid items-center gap-10 lg:grid-cols-[2fr_3fr]">
              <ol className="relative space-y-6 before:absolute before:inset-y-4 before:start-[1.1rem] before:w-px before:bg-gradient-to-b before:from-cyan-500/60 before:via-violet-500/60 before:to-emerald-500/60">
                {(["pick", "write", "ship"] as const).map((k, i) => (
                  <li key={k} className={`reveal relative flex gap-4 ${["acc-cyan", "acc-violet", "acc-green"][i]}`} style={{ animationDelay: `${i * 80}ms` }}>
                    <span className="tile relative size-9 shrink-0 rounded-full bg-card font-mono text-sm font-semibold tabular-nums">{i + 1}</span>
                    <div>
                      <h3 className="font-semibold">{t(`how.${k}.title`)}</h3>
                      <p className="mt-1 text-sm leading-relaxed text-muted-foreground">{t(`how.${k}.body`)}</p>
                    </div>
                  </li>
                ))}
              </ol>
              <div className="reveal min-w-0">
                <LangMorph />
              </div>
            </div>
          </div>
        </section>

        <FeatureGroup g={GROUPS.app} t={t} />
        <FeatureGroup g={GROUPS.web} t={t} wash />
        <FeatureGroup g={GROUPS.beyond} t={t} />

        <section id="demo" aria-labelledby="demo-title" className="acc-green section-wash relative scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead center id="demo-title" eyebrow={t("demo.eyebrow")} title={t("demo.title")} subtitle={t("demo.subtitle")} />
            <div className="mt-10">
              <HomeDemo />
            </div>
            <p className="mt-3 text-center text-xs text-muted-foreground">{t("demo.caption")}</p>
          </div>
        </section>

        <FeatureGroup g={GROUPS.grow} t={t} />

        <section id="results" aria-labelledby="results-title" className="acc-green section-wash scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="results-title" eyebrow={t("results.eyebrow")} title={t("results.title")} subtitle={t("results.subtitle")} />
            <figure className="reveal card-acc mt-10 rounded-xl border bg-card p-5 shadow-xs sm:p-7">
              <figcaption className="mb-2 inline-flex rounded-full border border-dashed px-2 py-0.5 text-[11px] font-medium text-muted-foreground">{t("results.example")}</figcaption>
              <blockquote className="text-lg font-medium text-pretty sm:text-2xl">“{t("results.exampleSentence")}”</blockquote>
            </figure>
            <div className="reveal mt-6">
              <WinnerBoard />
            </div>
          </div>
        </section>

        <FeatureGroup g={GROUPS.trust} t={t} />

        <section aria-labelledby="faq-title" className="mx-auto grid max-w-6xl gap-10 px-4 pb-24 lg:grid-cols-[1fr_2fr]">
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
          <div className="reveal card-acc acc-amber flex flex-col items-start gap-5 rounded-2xl border bg-card p-6 shadow-xs sm:p-8 md:flex-row md:items-center">
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
          <div aria-hidden className="aurora pointer-events-none top-auto! -bottom-1/2">
            <span />
            <span />
            <span />
            <span />
          </div>
          <div className="reveal relative mx-auto flex max-w-3xl flex-col items-center gap-5 px-4 py-24 text-center">
            <h2 id="cta-title" className="text-3xl font-semibold tracking-tight text-balance sm:text-5xl">{t("cta.title")}</h2>
            <p className="text-muted-foreground">{t("cta.body")}</p>
            {cta}
            <Link href="/docs" className="inline-flex min-h-6 items-center text-sm text-muted-foreground underline underline-offset-2 hover:text-foreground">
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
