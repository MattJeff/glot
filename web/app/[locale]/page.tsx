import {
  ArrowRight,
  BookOpenText,
  Bot,
  Check,
  Compass,
  EyeOff,
  FileClock,
  Globe,
  Link2,
  type LucideIcon,
  Mail,
  MapPin,
  MousePointerClick,
  Plug,
  RefreshCw,
  Rocket,
  Search,
  ShieldCheck,
  ShoppingBag,
  Sparkles,
  Unlock,
  Wand2,
  Zap,
  X,
} from "lucide-react";
import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { HomeDemo } from "@/components/home-demo";
import { APP_URL, type Chrome, getRepoStats, NPM_URL, SectionHead, SiteFooter, SiteHeader } from "@/components/landing";
import { ACCENTS } from "@/components/landing-accents";
import { WowShowcase } from "@/components/wow-showcase";
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

// Every feature, in plain words (the technical detail lives in /docs), ranked by what it does for glot:
// wow (shown live in the showcase), what sets it apart, what spreads it. Item keys under home.features.
type Group = { id: "diff" | "viral"; anchor: string; acc: string; items: [string, LucideIcon][] };
const DIFF: Group = {
  id: "diff",
  anchor: "different",
  acc: "acc-violet",
  items: [["app.safe", ShieldCheck], ["web.local", MapPin], ["grow.next", Compass], ["web.google", Search], ["app.brand", BookOpenText], ["beyond.emails", Mail], ["beyond.once", Link2], ["web.shop", ShoppingBag], ["web.longtail", Globe], ["grow.private", EyeOff], ["trust.proof", FileClock]],
};
const VIRAL: Group = {
  id: "viral",
  anchor: "together",
  acc: "acc-cyan",
  items: [["web.tag", Wand2], ["app.onpage", MousePointerClick], ["app.fresh", RefreshCw], ["app.agents", Bot], ["trust.open", Unlock]],
};
const WOW = [["minutes", "app"], ["ai", "beyond"], ["mobile", "beyond"], ["support", "beyond"], ["words", "grow"], ["live", "web"]] as const;
type Key = Parameters<T>[0];

// Under the hero: five promises, worded as the features they come from.
const TRUST = [["trust.open", Unlock, "acc-green"], ["app.safe", ShieldCheck, "acc-violet"], ["web.google", Search, "acc-cyan"], ["grow.private", EyeOff, "acc-amber"], ["app.minutes", Zap, "acc-coral"]] as const;

// Greetings orbiting the globe: the product's sample copy in each language, not UI text.
const ORBIT = [
  ["EN", "Hello!", "top-[4%] start-[30%]", "acc-violet"],
  ["DE", "Hallo!", "top-[20%] start-[4%]", "acc-amber"],
  ["IT", "Ciao!", "top-[42%] -start-[4%]", "acc-coral"],
  ["PT", "Olá!", "top-[62%] start-[4%]", "acc-green"],
  ["FR", "Bonjour !", "top-[6%] -end-[2%]", "acc-cyan"],
  ["JA", "こんにちは！", "top-[28%] -end-[6%]", "acc-pink"],
  ["ES", "¡Hola!", "top-[50%] -end-[4%]", "acc-violet"],
] as const;

function Globe3() {
  return (
    <div aria-hidden className="relative mx-auto aspect-square w-full max-w-[26rem] lg:max-w-[34rem]" dir="ltr">
      {/* Glowing ribbon around the globe; the moving dash only runs without reduced motion. */}
      <svg viewBox="0 0 400 400" className="absolute inset-0 size-full overflow-visible" fill="none">
        <defs>
          <linearGradient id="ribbon" x1="0" x2="1">
            <stop offset="0" stopColor="#22d3ee" />
            <stop offset=".5" stopColor="#a855f7" />
            <stop offset="1" stopColor="#f59e0b" />
          </linearGradient>
        </defs>
        <ellipse cx="200" cy="215" rx="215" ry="70" transform="rotate(-14 200 215)" stroke="url(#ribbon)" strokeWidth="2.5" opacity=".55" />
        <ellipse cx="200" cy="215" rx="215" ry="70" transform="rotate(-14 200 215)" stroke="white" strokeWidth="3" className="how-flow" pathLength="1460" />
      </svg>
      {/* Generated with Higgsfield (gpt_image_2_6), text-free so every locale shares it. Already a sized WebP. */}
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img src="/landing/globe.webp" alt="" width={880} height={880} fetchPriority="high" decoding="async" className="glow-img globe-float size-full" />
      {ORBIT.map(([code, hi, pos, acc], i) => (
        <span key={code} className={`${acc} chip-float glass absolute ${pos} flex items-center gap-2 rounded-full px-3 py-1.5 text-sm font-semibold`} style={{ animationDelay: `${i * -0.9}s` }}>
          <span className="tile rounded-md px-1.5 font-mono text-[10px]">{code}</span>
          {hi}
        </span>
      ))}
      <div className="acc-amber glass absolute start-1/2 bottom-[4%] w-[78%] -translate-x-1/2 rounded-2xl p-4 text-start">
        <div className="flex items-center justify-between">
          <span className="hello acc-ink inline-flex items-center gap-1.5 !py-0.5 text-xs font-semibold"><Sparkles className="size-3.5" /> AI</span>
          <span className="acc-green tile size-6 rounded-full"><Check className="size-3.5" /></span>
        </div>
        <div className="mt-3 grid grid-cols-[1fr_auto_1fr] items-center gap-3 text-sm">
          <p><span className="block font-mono text-[10px] text-muted-foreground">EN</span><i>Launch your product globally.</i></p>
          <span className="acc-green tile size-7 rounded-full"><ArrowRight className="size-4" /></span>
          <p><span className="block font-mono text-[10px] text-muted-foreground">FR</span>Lancez votre produit dans le monde entier.</p>
        </div>
      </div>
    </div>
  );
}

const FAQ = ["dev", "quality", "tools", "google", "privacy", "price"] as const;

type T = Awaited<ReturnType<typeof getTranslations<"home">>>;

function FeatureGroup({ g, t, wash, extra }: { g: Group; t: T; wash?: boolean; extra?: React.ReactNode }) {
  const k = (s: string) => `features.${s}` as Parameters<T>[0];
  return (
    <section id={g.anchor} aria-labelledby={`${g.anchor}-title`} className={`${g.acc} scroll-mt-16 ${wash ? "section-wash border-y bg-muted/30" : ""}`}>
      <div className="mx-auto max-w-6xl px-4 py-24">
        <SectionHead id={`${g.anchor}-title`} eyebrow={t(k(`${g.id}.eyebrow`))} title={t(k(`${g.id}.title`))} subtitle={t(k(`${g.id}.subtitle`))} />
        <ul className="mt-12 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {g.items.map(([id, Icon], i) => (
            // ponytail: 5 or 11 cards fill a 3-column grid when the first one spans two.
            <li key={id} className={`reveal card-acc flex flex-col ${g.items.length % 3 === 2 && i === 0 ? "lg:col-span-2" : ""} rounded-xl border bg-card p-5 shadow-xs ${ACCENTS[(ACCENTS.indexOf(g.acc as (typeof ACCENTS)[number]) + i) % ACCENTS.length]}`} style={{ animationDelay: `${(i % 3) * 60}ms` }}>
              <span className="tile size-10">
                <Icon aria-hidden className="size-5" />
              </span>
              <h3 className="mt-4 font-semibold">{t(k(`${id}.title`))}</h3>
              {i === 0 && extra}
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
    { href: "#wow", label: t("nav.wow") },
    { href: "#different", label: t("nav.different") },
    { href: "#demo", label: t("nav.demo") },
    { href: "#together", label: t("nav.together") },
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
        <a href="#wow">{t("hero.see")}</a>
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
          <div aria-hidden className="bg-grid pointer-events-none absolute inset-0 [mask-image:radial-gradient(ellipse_at_top,black,transparent_65%)] opacity-40" />
          <div className="relative mx-auto grid max-w-6xl items-center gap-8 px-4 pt-14 pb-10 sm:pt-20 lg:grid-cols-[1.05fr_1fr] lg:gap-4">
            <div className="text-center lg:text-start">
              <p className="mb-6 flex items-center justify-center gap-3 text-xs font-medium tracking-[0.2em] text-muted-foreground uppercase lg:justify-start"><span aria-hidden className="h-px w-8 bg-emerald-400" />{t("hero.badge")}</p>
              <h1 id="hero-title" className="text-5xl leading-[0.95] font-bold tracking-tighter text-balance sm:text-7xl">
                {t.rich("hero.title", { hl: (c) => <span className="hl">{c}</span> })}
              </h1>
              {/* No entrance animation on the text: it is the LCP. */}
              <p className="mx-auto mt-5 max-w-xl text-base text-pretty text-muted-foreground sm:text-lg lg:mx-0">{t("hero.subtitle")}</p>
              <div className="mt-8 [&>div]:lg:justify-start">{cta}</div>
              <ul aria-label={t("hero.proofLabel")} className="mt-8 flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-sm text-muted-foreground lg:justify-start">
                <li>
                  <Link href="/docs/benchmark" title={t("hero.proofBenchNote")} className="inline-flex min-h-6 items-center gap-1.5 hover:text-foreground">
                    <Check aria-hidden className="size-4 text-emerald-500" />
                    <span className="tabular-nums">{t("hero.proofBench")}</span>
                  </Link>
                </li>
                <li>
                  <a href={GITHUB_URL} target="_blank" rel="noreferrer" className="inline-flex min-h-6 items-center gap-1.5 hover:text-foreground">
                    <Check aria-hidden className="size-4 text-emerald-500" /> <span className="tabular-nums">{t("hero.proofStars", { count: stars ?? 0 })}</span>
                  </a>
                </li>
              </ul>
            </div>
            <Globe3 />
          </div>
          <div className="relative mx-auto max-w-6xl px-4 pb-6">
            <ul className="reveal glass grid grid-cols-2 overflow-hidden rounded-3xl sm:grid-cols-3 lg:grid-cols-5 lg:divide-x lg:rounded-full rtl:lg:divide-x-reverse">
              {TRUST.map(([key, Icon, acc]) => (
                <li key={key} className={`${acc} flex items-center gap-3 px-5 py-4`}>
                  <Icon aria-hidden className="acc-ink size-7 shrink-0" strokeWidth={1.6} />
                  <span className="text-sm leading-snug font-medium">{t(`features.${key}.title` as Key)}</span>
                </li>
              ))}
            </ul>
            <p className="mt-2 text-center text-[11px] text-muted-foreground">{t("hero.proofBenchNote")}</p>
          </div>
        </section>

        <HelloMarquee />

        <section id="wow" aria-labelledby="wow-title" className="scroll-mt-16">
          <div className="mx-auto max-w-6xl px-4 pt-12 pb-24">
            <SectionHead id="wow-title" eyebrow={t("features.wow.eyebrow")} title={t("features.wow.title")} subtitle={t("features.wow.subtitle")} />
            <div className="mt-12">
              <WowShowcase label={t("features.wow.title")} items={WOW.map(([id, g]) => ({ id, title: t(`features.${g}.${id}.title` as Key), gain: t(`features.${g}.${id}.gain` as Key) }))} />
            </div>
          </div>
        </section>

        <section id="how" aria-labelledby="how-title" className="relative scroll-mt-16 overflow-hidden border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead center id="how-title" eyebrow={t("how.eyebrow")} title={t("how.title")} />
            <div className="relative mt-16">
              <svg aria-hidden viewBox="0 0 1200 120" preserveAspectRatio="none" className="how-wave pointer-events-none absolute inset-x-0 top-0 hidden h-28 w-full md:block">
                <defs>
                  <linearGradient id="how-g" x1="0" x2="1">
                    <stop offset="0" stopColor="#22d3ee" />
                    <stop offset="0.5" stopColor="#a855f7" />
                    <stop offset="1" stopColor="#10b981" />
                  </linearGradient>
                </defs>
                <path d="M0 60 C 150 0, 250 120, 400 60 S 650 0, 800 60 S 1050 120, 1200 60" fill="none" stroke="url(#how-g)" strokeWidth="3" />
                <path className="how-flow" d="M0 60 C 150 0, 250 120, 400 60 S 650 0, 800 60 S 1050 120, 1200 60" fill="none" stroke="white" strokeWidth="3" strokeLinecap="round" />
              </svg>
              <ol className="relative grid gap-10 md:grid-cols-3">
                {([["pick", Plug, "acc-cyan"], ["write", Sparkles, "acc-violet"], ["ship", Rocket, "acc-green"]] as const).map(([k, Icon, acc], i) => (
                  <li key={k} className={`reveal ${acc} flex flex-col items-center text-center`} style={{ animationDelay: `${i * 100}ms` }}>
                    <span className="how-node tile size-16 rounded-full bg-card">
                      <Icon aria-hidden className="size-6" />
                    </span>
                    <h3 className="mt-5 text-lg font-semibold">
                      <span className="acc-ink tabular-nums">{i + 1}.</span> {t(`how.${k}.title`)}
                    </h3>
                    <p className="mt-2 max-w-xs text-sm leading-relaxed text-muted-foreground">{t(`how.${k}.body`)}</p>
                  </li>
                ))}
              </ol>
            </div>
          </div>
        </section>

        <FeatureGroup
          g={DIFF}
          t={t}
          extra={
            // Benchmark: docs/benchmark (320 sentences, 8 languages, same model with and without glot).
            <div aria-hidden className="mt-4 space-y-2 font-mono text-xs" dir="ltr">
              <div className="flex items-center gap-3">
                <span className="w-12 text-muted-foreground">AI</span>
                <div className="h-3 flex-1 overflow-hidden rounded-full bg-muted">
                  <div className="bench-bar h-full w-[100%] rounded-full bg-red-500/70" />
                </div>
                <span className="w-12 text-end tabular-nums">14.1%</span>
              </div>
              <div className="flex items-center gap-3">
                <span className="w-12 font-semibold">glot</span>
                <div className="h-3 flex-1 rounded-full bg-muted" />
                <span className="acc-ink w-12 text-end font-semibold tabular-nums">0%</span>
              </div>
            </div>
          }
        />

        <section id="demo" aria-labelledby="demo-title" className="acc-green section-wash relative scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead center id="demo-title" eyebrow={t("demo.eyebrow")} title={t("demo.title")} subtitle={t("demo.subtitle")} />
            <div className="mt-10">
              <HomeDemo />
            </div>
            <p className="mt-3 text-center text-xs text-muted-foreground">{t("demo.caption")}</p>
          </div>
        </section>

        <FeatureGroup g={VIRAL} t={t} />

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
          {/* Generated with Higgsfield (gpt_image_2_6), text-free so every locale shares it. Already a sized WebP. */}
      {/* eslint-disable-next-line @next/next/no-img-element */}
          <img aria-hidden src="/landing/waves.webp" alt="" width={1344} height={576} loading="lazy" decoding="async" className="glow-img pointer-events-none absolute inset-x-0 bottom-0 w-full opacity-80" />
          <div className="reveal relative mx-auto flex max-w-3xl flex-col items-center gap-5 px-4 py-24 text-center">
            <h2 id="cta-title" className="text-3xl font-semibold tracking-tight text-balance sm:text-5xl">{t.rich("cta.title", { hl: (c) => <span className="hl">{c}</span> })}</h2>
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
