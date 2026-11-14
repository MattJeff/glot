import { ArrowRight, Check, Play, X } from "lucide-react";
import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { HomeDemo } from "@/components/home-demo";
import { APP_URL, type Chrome, getRepoStats, NPM_URL, SectionHead, SiteFooter, SiteHeader } from "@/components/landing";
import { ACCENTS } from "@/components/landing-accents";
import { Art } from "@/components/landing-art";
import { DemoScene, HowScene, ProblemScene, ResultsScene } from "@/components/landing-scenes";
import { WowShowcase } from "@/components/wow-showcase";
import { PricingCards } from "./pricing/pricing-cards";
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

// Every feature, in plain words (the technical detail lives in /docs), ranked by what it does for glot:
// wow (shown live in the showcase), what sets it apart, what spreads it. Item keys under home.features.
type Group = { id: "diff" | "viral"; anchor: string; acc: string; items: [string, string][] };
const DIFF: Group = {
  id: "diff",
  anchor: "different",
  acc: "acc-violet",
  items: [["app.safe", "shield"], ["web.local", "pin"], ["grow.next", "compass"], ["web.google", "search"], ["app.brand", "book"], ["beyond.emails", "mail"], ["beyond.once", "link"], ["web.shop", "bag"], ["web.longtail", "globe"], ["grow.private", "eyeoff"], ["trust.proof", "fileclock"]],
};
const VIRAL: Group = {
  id: "viral",
  anchor: "together",
  acc: "acc-cyan",
  items: [["web.tag", "wand"], ["app.onpage", "pointer"], ["app.fresh", "refresh"], ["app.agents", "bot"], ["trust.open", "unlock"]],
};
const WOW = [["minutes", "app"], ["ai", "beyond"], ["mobile", "beyond"], ["support", "beyond"], ["words", "grow"], ["live", "web"]] as const;
type Key = Parameters<T>[0];

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

function Globe3({ plates }: { plates: string[] }) {
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
        <span key={code} className={`${acc} chip-float plate absolute ${pos} flex items-center gap-2 rounded-full px-3 py-1.5 text-sm font-semibold`} style={{ animationDelay: `${i * -0.9}s` }}>
          {/* Flags generated with Higgsfield (gpt_image_2_6), cropped to 96 px WebP. */}
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={`/landing/flags/${code.toLowerCase()}.webp`} alt="" width={24} height={24} className="size-6 rounded-full shadow-sm" />
          {hi}
        </span>
      ))}
      <span className="acc-amber plate absolute start-1/2 top-[40%] flex -translate-x-1/2 -translate-y-1/2 -rotate-6 items-center gap-2 rounded-2xl px-5 py-3 text-3xl font-bold shadow-[0_0_60px_-10px_#ff9a3c]">
        <span className="text-[#ff9a3c]">{"{ }"}</span> glot
      </span>
      <div className="acc-amber plate plate-lit absolute start-1/2 bottom-[2%] w-[70%] -translate-x-1/2 rounded-2xl p-4 text-start">
        <div className="flex items-center justify-between">
          <span className="acc-amber plate inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-semibold"><Art n="sparkles" className="size-4" /> {plates[1]}</span>
          <span className="grid size-6 place-items-center rounded-full bg-[#2ef7b7] text-black"><Check className="size-3.5" /></span>
        </div>
        <div className="mt-3 grid grid-cols-[1fr_auto_1fr] items-center gap-3 text-sm">
          <p><span className="block font-mono text-[10px] text-muted-foreground">EN</span><i>Launch your product globally.</i></p>
          <span className="grid size-7 place-items-center rounded-full border border-[#2ef7b7]/50 text-[#2ef7b7]"><ArrowRight className="size-4" /></span>
          <p><span className="block font-mono text-[10px] text-muted-foreground">FR</span>Lancez votre produit dans le monde entier.</p>
        </div>
      </div>
      {([[0, "layers", "acc-green", "bottom-[8%] -start-[8%] -rotate-[10deg]"], [2, "book", "acc-violet", "bottom-[6%] -end-[6%] rotate-[6deg]"], [3, "zap", "acc-amber", "bottom-[24%] -end-[14%] rotate-[8deg]"]] as const).map(([i, icon, acc, pos]) => (
        <span key={i} className={`${acc} plate absolute hidden w-36 items-center gap-2 rounded-xl p-3 text-xs leading-tight font-semibold xl:flex ${pos}`}>
          <Art n={icon} className="size-8 shrink-0" /> {plates[i]}
        </span>
      ))}
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
              <Art n={Icon} className="size-14 -ms-1" />
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
    { href: "#why", label: t("nav.different") },
    { href: "#how", label: t("how.eyebrow") },
    { href: "#demo", label: t("nav.demo") },
    { href: "#wow", label: t("nav.wow") },
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

  const s = await getTranslations("home.scenes");
  const pct = new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: 1 });
  const num = new Intl.NumberFormat(locale);
  // Only verifiable figures: benchmark (docs/benchmark), licence, live GitHub stars, no-cookie analytics.
  const stats: [string, string, string, string, string][] = [
    [pct.format(0), s("hero.statBroken"), "shield", "acc-green", "/docs/benchmark"],
    [pct.format(0.141), s("hero.statAlone"), "chart", "acc-amber", "/docs/benchmark"],
    ["MIT", s("hero.statOss"), "unlock", "acc-violet", GITHUB_URL],
    [num.format(stars ?? 0), s("hero.statStars"), "trophy", "acc-amber", GITHUB_URL],
    [num.format(0), s("hero.statCookie"), "eyeoff", "acc-cyan", "/docs/faq"],
  ];
  const metrics: [string, string, string, string][] = [
    [pct.format(0), s("results.m1"), "shield", "acc-green"],
    [pct.format(0.141), s("results.m2"), "chart", "acc-amber"],
    [num.format(320), s("results.m3"), "languages", "acc-violet"],
    [num.format(6), s("results.m4"), "globe", "acc-cyan"],
  ];

  const cta = (
    <div className="flex flex-col items-center justify-center gap-3 sm:flex-row">
      <a href={`${APP_URL}/signup`} className="inline-flex h-12 items-center gap-2 rounded-full bg-[#ff9a3c] px-6 font-semibold whitespace-nowrap text-black shadow-[0_0_30px_-6px_#ff9a3c] transition-transform hover:-translate-y-0.5">
        {s("hero.start")} <ArrowRight aria-hidden className="size-4 rtl:rotate-180" />
      </a>
      <a href="#demo" className="inline-flex h-12 items-center gap-2 rounded-full border border-current/30 px-6 font-semibold whitespace-nowrap hover:bg-foreground/5">
        {s("hero.demo")} <Play aria-hidden className="size-4 fill-current" />
      </a>
    </div>
  );

  return (
    <div className="flex min-h-screen flex-col">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(ld).replace(/</g, "\\u003c") }} />
      <SiteHeader c={chrome} links={navLinks} stars={stars} />

      <main id="main" className="flex-1">
        <section aria-labelledby="hero-title" className="scene relative overflow-hidden bg-background text-foreground">
          <div className="relative mx-auto grid max-w-7xl items-center gap-8 px-4 pt-14 pb-10 sm:pt-20 lg:grid-cols-[0.9fr_1.3fr] lg:gap-4">
            <div className="text-center lg:text-start">
              <p className="mb-6 flex items-center justify-center gap-3 text-xs font-medium tracking-[0.25em] text-muted-foreground uppercase lg:justify-start">
                <span aria-hidden className="h-px w-8 bg-[#2ef7b7]" />
                {s("hero.eyebrow")}
              </p>
              <h1 id="hero-title" className="text-6xl leading-[0.9] font-extrabold tracking-[-0.04em] sm:text-8xl">
                <span className="block">{s("hero.l1")}</span>
                <span className="block">{s("hero.l2")}</span>
                <span className="o-hl block">{s("hero.l3")}</span>
                <span className="m-hl block">{s("hero.l4")}</span>
              </h1>
              {/* No entrance animation on the text: it is the LCP. */}
              <p className="mx-auto mt-6 max-w-xl text-lg text-pretty text-muted-foreground lg:mx-0">{s("hero.subtitle")}</p>
              <div className="mt-8 flex flex-col items-center gap-3 sm:flex-row lg:justify-start">
                <a href={`${APP_URL}/signup`} className="mint-btn inline-flex h-14 items-center gap-2 rounded-full px-7 text-base font-semibold transition-transform hover:-translate-y-0.5">
                  {s("hero.start")} <ArrowRight aria-hidden className="size-5 rtl:rotate-180" />
                </a>
                <a href="#demo" className="inline-flex h-14 items-center gap-2 rounded-full border border-foreground/25 px-7 text-base font-semibold hover:bg-foreground/5">
                  <Play aria-hidden className="size-4 fill-current" /> {s("hero.demo")}
                </a>
              </div>
              <ul className="mt-6 flex flex-wrap items-center justify-center gap-x-5 gap-y-2 text-sm text-muted-foreground lg:justify-start">
                {(["check1", "check2", "check3"] as const).map((k) => (
                  <li key={k} className="flex items-center gap-1.5">
                    <Check aria-hidden className="size-4 text-[#2ef7b7]" /> {s(`hero.${k}`)}
                  </li>
                ))}
              </ul>
            </div>
            <Globe3 plates={[s("hero.plateCulture"), s("hero.plateAi"), s("hero.plateBrand"), s("hero.platePublish")]} />
          </div>
          <div className="relative mx-auto max-w-7xl px-4 pb-10">
            <ul className="plate grid grid-cols-2 overflow-hidden rounded-3xl sm:grid-cols-3 lg:grid-cols-5 lg:divide-x lg:divide-foreground/10 lg:rounded-full rtl:lg:divide-x-reverse">
              {stats.map(([v, label, icon, acc, href]) => (
                <li key={label} className={`${acc} px-6 py-4`}>
                  <a href={href} className="flex items-center gap-3 hover:opacity-90">
                    <Art n={icon} className="size-10 shrink-0" />
                    <span>
                      <span className="block text-2xl leading-none font-bold tabular-nums">{v}</span>
                      <span className="block text-sm text-muted-foreground">{label}</span>
                    </span>
                  </a>
                </li>
              ))}
            </ul>
            <p className="mt-3 text-center text-[11px] text-muted-foreground">{s("hero.statNote")}</p>
          </div>
        </section>

        <ProblemScene s={s} cta={cta} />

        <HowScene s={s} />

        <section id="wow" aria-labelledby="wow-title" className="scroll-mt-16">
          <div className="mx-auto max-w-6xl px-4 py-24">
            <SectionHead id="wow-title" eyebrow={t("features.wow.eyebrow")} title={t("features.wow.title")} subtitle={t("features.wow.subtitle")} />
            <div className="mt-12">
              <WowShowcase label={t("features.wow.title")} items={WOW.map(([id, g]) => ({ id, title: t(`features.${g}.${id}.title` as Key), gain: t(`features.${g}.${id}.gain` as Key) }))} />
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

        <DemoScene s={s} demo={<HomeDemo />} />

        <FeatureGroup g={VIRAL} t={t} />

        <ResultsScene
          s={s}
          metrics={metrics}
          plans={<PricingCards />}
          cta={cta}
          faq={
            <div className="space-y-3">
              {faq.map((f, i) => (
                <details key={f.q} open={i === 0} className="faq group plate rounded-2xl open:border-[#ff9a3c]/70">
                  <summary className="flex min-h-14 cursor-pointer list-none items-center justify-between gap-4 px-5 py-4 font-semibold focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
                    {f.q}
                    <span aria-hidden className="grid size-6 shrink-0 place-items-center rounded-full border border-foreground/20 text-muted-foreground transition-transform duration-200 group-open:rotate-45">
                      +
                    </span>
                  </summary>
                  <p className="px-5 pb-5 text-sm leading-relaxed text-muted-foreground">{f.a}</p>
                </details>
              ))}
              <Link href="/docs/faq" className="inline-flex min-h-6 items-center gap-1 text-sm text-muted-foreground underline underline-offset-2 hover:text-foreground">
                {t("faq.more")}
              </Link>
            </div>
          }
        />
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
