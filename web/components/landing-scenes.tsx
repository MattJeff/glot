import { ArrowRight, Check, Play, X } from "lucide-react";
import type { getTranslations } from "next-intl/server";
import { Art } from "@/components/landing-art";
import { Link } from "@/i18n/navigation";

// The illustrated scenes of the landing (mockups 02–05). Each is a `.dark` band whatever the site theme:
// the art is lit on black. Illustrations: Higgsfield (gpt_image_2_6), WebP in /public/landing.
type S = Awaited<ReturnType<typeof getTranslations<"home.scenes">>>;

/* eslint-disable @next/next/no-img-element -- pre-sized decorative WebP, no optimizer needed */
// Cut-outs (transparent WebP) need no blending; scenery on black uses .glow-img or is dark-only.
const CUT = new Set(["robot-wave", "robot-tired", "robot-fly", "robot-panels", "robot-chart", "pedestal", "tile-glot", "tile-faq"]);
function Img({ src, className = "" }: { src: string; className?: string }) {
  const look = CUT.has(src) ? "drop-shadow-[0_18px_30px_rgb(0_0_0/0.25)]" : src === "ribbon" ? "glow-img" : "scenery";
  return <img src={`/landing/${src}.webp`} alt="" aria-hidden loading="lazy" decoding="async" className={`${look} pointer-events-none select-none ${className}`} />;
}

export function Badge({ children, acc = "acc-amber", n }: { children: React.ReactNode; acc?: string; n?: string }) {
  return (
    <p className={`${acc} plate inline-flex items-center gap-2 rounded-full px-3 py-1 text-sm font-medium`}>
      {n && <span className="acc-ink font-semibold tabular-nums">{n}</span>}
      {children}
    </p>
  );
}

function Title({ a, hl, tone = "o", className = "" }: { a: string; hl: string; tone?: "o" | "m"; className?: string }) {
  return (
    <h2 className={`text-4xl leading-[1.02] font-bold tracking-tight text-balance sm:text-5xl ${className}`}>
      {a} <span className={tone === "o" ? "o-hl" : "m-hl"}>{hl}</span>
    </h2>
  );
}

const PROBLEMS = [["p1", "languages"], ["p2", "layers"], ["p3", "users"], ["p4", "chart"]] as const;
const SOLUTIONS = [["s1", "acc-green", "zap"], ["s2", "acc-amber", "pin"], ["s3", "acc-violet", "book"], ["s4", "acc-cyan", "trophy"]] as const;

/** Mockup 02: the climb without glot, then the ribbon with it. */
export function ProblemScene({ s, cta }: { s: S; cta: React.ReactNode }) {
  return (
    <section id="why" aria-labelledby="why-title" className="scene relative scroll-mt-16 overflow-hidden bg-background text-foreground">
      <div className="relative mx-auto max-w-7xl px-4 pt-20 pb-10">
        <div className="grid items-start gap-10 lg:grid-cols-[0.8fr_1.6fr]">
          <div>
            <Badge n="02">{s("problem.badge")}</Badge>
            <Title a={s("problem.title")} hl={s("problem.titleHl")} className="mt-5" />
            <p className="mt-5 max-w-md text-lg text-muted-foreground">{s("problem.body")}</p>
            <Img src="robot-tired" className="mt-6 hidden w-44 lg:block" />
          </div>
          <div className="relative">
            <div className="flex flex-wrap items-center gap-4">
              <span className="acc-pink plate inline-flex items-center gap-2 rounded-full px-4 py-2 font-semibold">
                <X aria-hidden className="acc-ink size-5" /> {s("problem.without")}
              </span>
              <span className="text-muted-foreground">{s("problem.withoutBody")}</span>
            </div>
            <div className="relative mt-4 pb-[18%]">
              <Img src="mountain" className="fade-x absolute inset-x-0 top-10 w-full dark:-top-6" />
              <ol className="relative grid grid-cols-2 gap-3 pt-6 sm:grid-cols-4 sm:pt-24">
                {PROBLEMS.map(([k, icon], i) => (
                  <li key={k} className="acc-amber plate reveal rounded-2xl p-4 sm:[&:nth-child(2)]:-mt-8 sm:[&:nth-child(3)]:-mt-16 sm:[&:nth-child(4)]:-mt-24" style={{ animationDelay: `${i * 90}ms` }}>
                    <span className="flex items-center justify-between">
                      <span className="text-3xl font-bold text-foreground/25 tabular-nums">0{i + 1}</span>
                      <Art n={icon} className="size-9" />
                    </span>
                    <p className="mt-2 leading-snug font-semibold">{s(`problem.${k}`)}</p>
                    <p className="acc-ink mt-2 text-xs">{s(`problem.${k}s`)}</p>
                  </li>
                ))}
              </ol>
            </div>
            <p className="hand mt-6 text-end text-xl text-foreground/80">{s("problem.verdict")}</p>
          </div>
        </div>

        {/* The hinge: glot between the two worlds. */}
        <div aria-hidden className="relative mx-auto my-10 flex w-fit items-center justify-center">
          <Img src="tile-glot" className="w-40" />
          {(["tagTranslate", "tagLocalize", "tagCheck", "tagMeasure"] as const).map((k, i) => (
            <span
              key={k}
              className={`${["acc-violet", "acc-amber", "acc-green", "acc-violet"][i]} plate absolute rounded-lg px-3 py-1 text-sm font-semibold whitespace-nowrap ${["-start-24 top-4", "-start-28 bottom-6", "-end-24 top-2", "-end-24 bottom-8"][i]}`}
            >
              {s(`problem.${k}`)} ↗
            </span>
          ))}
        </div>

        <div className="grid items-end gap-10 lg:grid-cols-[0.8fr_1.6fr]">
          <div>
            <Title a={s("problem.solTitle")} hl={s("problem.solHl")} tone="m" />
            <p className="mt-5 max-w-md text-lg text-muted-foreground">{s("problem.solBody")}</p>
            <div className="mt-7 [&>div]:lg:justify-start">{cta}</div>
          </div>
          <div className="relative">
            <span className="acc-green plate absolute -top-16 end-0 z-10 inline-flex items-center gap-2 rounded-full px-4 py-2 font-semibold">
              <Check aria-hidden className="acc-ink size-5" /> {s("problem.with")}
            </span>
            <Img src="ribbon" className="fade-x absolute inset-x-0 -bottom-10 w-full opacity-80" />
            <Img src="robot-fly" className="absolute -start-24 top-0 hidden w-40 lg:block" />
            <ol className="relative grid grid-cols-2 gap-3 sm:grid-cols-4">
              {SOLUTIONS.map(([k, acc, icon], i) => (
                <li key={k} className={`${acc} plate plate-lit reveal rounded-2xl p-4 sm:[&:nth-child(1)]:mt-24 sm:[&:nth-child(2)]:mt-16 sm:[&:nth-child(3)]:mt-8`} style={{ animationDelay: `${i * 90}ms` }}>
                  <span className="flex items-center gap-2">
                    <Art n={icon} className="size-9" />
                    <span className="acc-ink text-3xl font-bold tabular-nums">0{i + 1}</span>
                  </span>
                  <p className="mt-3 text-base leading-snug font-semibold [overflow-wrap:anywhere] hyphens-auto sm:text-lg">{s(`problem.${k}`)}</p>
                </li>
              ))}
            </ol>
          </div>
        </div>

        <div className="relative mt-16 grid gap-6 lg:grid-cols-[1.4fr_1fr] lg:items-center">
          <ul className="plate grid grid-cols-3 divide-x divide-foreground/10 rounded-2xl rtl:divide-x-reverse">
            {(["fast", "simple", "profit"] as const).map((k, i) => (
              <li key={k} className="flex items-center gap-3 p-4">
                <Art n={["zap", "layers", "chart"][i]} className="hidden size-9 shrink-0 sm:block" />
                <span>
                  <span className="block text-sm font-semibold">{s(`problem.${k}`)}</span>
                  <span className="block text-xs text-muted-foreground">{s(`problem.${k}S`)}</span>
                </span>
              </li>
            ))}
          </ul>
          <p className="acc-amber plate flex items-center gap-3 rounded-2xl p-4">
            <Art n="rocket" className="size-10 shrink-0" />
            <span>
              <span className="font-semibold">{s("problem.result")}</span> <span className="text-muted-foreground">{s("problem.resultBody")}</span>
            </span>
          </p>
        </div>
      </div>
      <Img src="earth" className="fade-edges absolute hidden dark:block -end-20 bottom-0 w-[55%] opacity-70" />
    </section>
  );
}

const STEPS = [
  ["1", "acc-amber", "plug"],
  ["2", "acc-violet", "sparkles"],
  ["3", "acc-violet", "book"],
  ["4", "acc-pink", "rocket"],
  ["5", "acc-green", "chart"],
] as const;
const SURFACES = [
  ["mobile", "acc-violet", "phone", "SDK"],
  ["web", "acc-cyan", "globe", "Widget"],
  ["email", "acc-green", "mail", "API"],
  ["support", "acc-amber", "chat", "connectors"],
] as const;

/** Mockup 03: five steps on a lit path, the robot on its pedestal, four product surfaces. */
export function HowScene({ s }: { s: S }) {
  return (
    <section id="how" aria-labelledby="how-title" className="scene relative scroll-mt-16 overflow-hidden bg-background text-foreground">
      <div className="relative mx-auto max-w-7xl px-4 py-20">
        <div className="flex flex-wrap items-end justify-between gap-8">
          <div>
            <Badge acc="acc-amber">{s("how.badge")}</Badge>
            <h2 id="how-title" className="mt-4 text-5xl font-bold tracking-tight sm:text-6xl">
              {s("how.title")} <span className="o-hl">{s("how.titleHl")}</span>
            </h2>
            <p className="mt-3 text-lg text-muted-foreground">{s("how.subtitle")}</p>
          </div>
          <ul className="grid grid-cols-3 gap-6 border-foreground/15 lg:border-s lg:ps-8">
            {(["fast", "complete", "scale"] as const).map((k, i) => (
              <li key={k} className="flex items-start gap-2">
                <Art n={["zap", "layers", "sparkles"][i]} className="size-9 shrink-0" />
                <span>
                  <span className="block font-semibold">{s(`how.${k}`)}</span>
                  <span className="block text-xs text-muted-foreground">{s(`how.${k}S`)}</span>
                </span>
              </li>
            ))}
          </ul>
        </div>

        <div className="mt-12 grid gap-10 lg:grid-cols-[1fr_0.7fr_1.5fr] lg:items-center">
          <ol className="relative space-y-4 border-s-2 border-[oklch(0.75_0.15_60/0.5)] ps-6">
            {STEPS.map(([n, acc, icon], i) => (
              <li key={n} className={`${acc} plate reveal relative flex items-start gap-4 rounded-2xl p-4 lg:[&:nth-child(even)]:ms-6`} style={{ animationDelay: `${i * 80}ms` }}>
                <span aria-hidden className="how-dot absolute top-1/2 -start-[2.15rem] size-3 -translate-y-1/2 rounded-full" />
                <span className="acc-ink text-3xl font-bold tabular-nums">0{n}</span>
                <Art n={icon} className="size-10 shrink-0" />
                <span>
                  <span className="acc-ink block text-xs font-semibold tracking-wider uppercase">{s(`how.k${n}`)}</span>
                  <span className="block font-semibold">{s(`how.t${n}`)}</span>
                  <span className="block text-sm text-muted-foreground">{s(`how.b${n}`)}</span>
                </span>
              </li>
            ))}
          </ol>

          <div aria-hidden className="relative mx-auto w-64 lg:w-full">
            <Img src="robot-panels" className="relative z-10 mx-auto w-4/5" />
            <Img src="pedestal" className="-mt-16 w-full" />
          </div>

          <div>
            <Badge acc="acc-amber">{s("how.surfaces")}</Badge>
            <h3 className="mt-4 text-3xl leading-tight font-bold tracking-tight sm:text-4xl">
              {s("how.surfTitle")} <span className="o-hl">{s("how.surfHl")}</span>
            </h3>
            <ul className="mt-8 grid gap-4 sm:grid-cols-2">
              {SURFACES.map(([k, acc, icon, tag], i) => (
                <li key={k} className={`${acc} plate plate-lit reveal rounded-3xl p-5 sm:[&:nth-child(even)]:translate-y-4`} style={{ animationDelay: `${i * 90}ms` }}>
                  <div className="flex items-center justify-between gap-2">
                    <span className="flex items-center gap-2">
                      <Art n={icon} className="size-11" />
                      <span className="text-xl font-bold">{s(`how.${k}`)}</span>
                    </span>
                    <span className="acc-ink rounded-full border border-current/40 px-2.5 py-0.5 text-xs font-semibold">{tag === "connectors" ? s("how.connectors") : tag}</span>
                  </div>
                  <p className="mt-3 text-sm text-muted-foreground">{s(`how.${k}B`)}</p>
                  <ul className="mt-3 space-y-1 text-sm">
                    {([1, 2, 3] as const).map((j) => (
                      <li key={j} className="flex items-center gap-2">
                        <Check aria-hidden className="acc-ink size-4 shrink-0" />
                        {s(`how.${k}${j}`)}
                      </li>
                    ))}
                  </ul>
                </li>
              ))}
            </ul>
          </div>
        </div>
      </div>
    </section>
  );
}

/** Mockup 04: the workshop. The live demo is the real component, tilted, with notes around it. */
export function DemoScene({ s, demo }: { s: S; demo: React.ReactNode }) {
  const notes = [
    ["n1", "acc-amber", "zap", "lg:-top-8 lg:start-[24%]"],
    ["n2", "acc-cyan", "layers", "lg:-top-6 lg:end-[12%]"],
    ["n3", "acc-green", "shield", "lg:bottom-[18%] lg:-start-6"],
    ["n4", "acc-amber", "trophy", "lg:top-[38%] lg:-end-6"],
  ] as const;
  return (
    <section id="demo" aria-labelledby="demo-title" className="scene relative scroll-mt-16 overflow-hidden bg-background text-foreground">
      <div className="relative mx-auto max-w-7xl px-4 py-20">
        <div className="grid gap-10 lg:grid-cols-[0.75fr_2fr]">
          <div className="lg:pt-10">
            <Badge n="04">{s("demo.badge")}</Badge>
            <h2 id="demo-title" className="mt-5 text-4xl leading-[1.05] font-bold tracking-tight sm:text-5xl">
              {s("demo.title")} <span className="o-hl not-italic">{s("demo.titleHl")}</span>
            </h2>
            <p className="mt-5 text-lg text-muted-foreground">{s("demo.body")}</p>
            <a href="#demo-live" className="mt-7 inline-flex h-12 items-center gap-2 rounded-full bg-[#ff9a3c] px-6 font-semibold text-black shadow-[0_0_30px_-6px_#ff9a3c] transition-transform hover:-translate-y-0.5">
              {s("demo.start")} <Play aria-hidden className="size-4 fill-current" />
            </a>
            <Link href="/docs" className="mt-4 flex items-center gap-2 text-sm underline underline-offset-4 hover:text-foreground">
              {s("demo.explore")} <ArrowRight aria-hidden className="size-4 rtl:rotate-180" />
            </Link>
          </div>
          <div className="relative">
            <Img src="robot-wave" className="absolute -end-10 -top-16 z-20 hidden w-40 lg:block" />
            <div id="demo-live" className="tilt relative z-10 scroll-mt-24 rounded-[1.75rem] border border-foreground/15 bg-card/80 p-2 shadow-[0_40px_120px_-30px_rgba(168,85,247,.55)] backdrop-blur">
              {demo}
            </div>
            <ul className="mt-6 grid gap-3 sm:grid-cols-2 lg:mt-0 lg:block">
              {notes.map(([k, acc, icon, pos]) => (
                <li key={k} className={`${acc} plate plate-lit z-20 flex items-start gap-3 rounded-2xl p-4 lg:absolute lg:w-60 ${pos}`}>
                  <Art n={icon} className="size-10 shrink-0" />
                  <span>
                    <span className="block font-semibold">{s(`demo.${k}`)}</span>
                    <span className="block text-sm text-muted-foreground">{s(`demo.${k}b`)}</span>
                  </span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      </div>
      <Img src="earth" className="fade-edges absolute hidden dark:block -end-10 -bottom-10 w-[45%] opacity-80" />
    </section>
  );
}

/** Mockup 05: verifiable numbers (no invented metrics), the real plans, the real FAQ, the final call. */
export function ResultsScene({ s, metrics, plans, faq, cta }: { s: S; metrics: [string, string, string, string][]; plans: React.ReactNode; faq: React.ReactNode; cta: React.ReactNode }) {
  return (
    <section aria-labelledby="results-title" className="scene relative overflow-hidden bg-background text-foreground">
      <div className="relative mx-auto max-w-7xl px-4 py-20">
        <div className="grid items-center gap-10 lg:grid-cols-[1fr_0.5fr_1.3fr]">
          <div>
            <Badge>{s("results.badge")}</Badge>
            <h2 id="results-title" className="mt-4 text-4xl leading-[1.05] font-bold tracking-tight sm:text-5xl">
              {s("results.title")} <span className="o-hl not-italic">{s("results.titleHl")}</span>
            </h2>
            <p className="mt-4 text-lg text-muted-foreground">{s("results.body")}</p>
            <ul className="mt-5 flex flex-wrap gap-x-5 gap-y-2 text-sm">
              {(["c1", "c2", "c3"] as const).map((k) => (
                <li key={k} className="flex items-center gap-1.5">
                  <Check aria-hidden className="size-4 text-[#ff9a3c]" /> {s(`results.${k}`)}
                </li>
              ))}
            </ul>
          </div>
          <Img src="robot-chart" className="mx-auto w-48 lg:w-full" />
          <div>
            <p className="acc-amber plate mx-auto mb-3 w-fit rounded-full px-3 py-1 text-sm font-medium">{s("results.impact")}</p>
            <ul className="plate grid grid-cols-2 divide-foreground/10 overflow-hidden rounded-3xl sm:grid-cols-4 sm:divide-x rtl:sm:divide-x-reverse">
              {metrics.map(([v, label, icon, acc]) => (
                <li key={label} className={`${acc} p-5 text-center`}>
                  <Art n={icon} className="mx-auto size-10" />
                  <p className="mt-2 text-2xl font-bold whitespace-nowrap tabular-nums xl:text-3xl">{v}</p>
                  <p className="mt-1 text-xs text-muted-foreground">{label}</p>
                </li>
              ))}
            </ul>
          </div>
        </div>

        <div id="pricing" className="mt-24 scroll-mt-20">
          <Badge>{s("results.plans")}</Badge>
          <h2 className="mt-4 text-4xl font-bold tracking-tight">
            {s("results.plansTitle")} <span className="o-hl not-italic">{s("results.plansHl")}</span>
          </h2>
          <div className="mt-8">{plans}</div>
        </div>

        <div className="mt-24 grid gap-10 lg:grid-cols-[0.6fr_1.4fr]">
          <div>
            <Badge>{s("results.faqBadge")}</Badge>
            <h2 className="mt-4 text-5xl font-bold tracking-tight">
              {s("results.faqTitle")} <span className="o-hl not-italic block">{s("results.faqHl")}</span>
            </h2>
            <p className="mt-3 text-muted-foreground">{s("results.faqBody")}</p>
            <Img src="tile-faq" className="mt-6 w-40" />
          </div>
          {faq}
        </div>

        <div className="plate relative mt-24 overflow-hidden rounded-[2.5rem] p-8 sm:p-12">
          <Img src="earth" className="fade-edges absolute hidden dark:block -end-10 -bottom-16 w-[70%] opacity-90" />
          <Img src="robot-wave" className="absolute end-8 top-1/2 hidden w-56 -translate-y-1/2 lg:block" />
          <div className="relative max-w-2xl">
            <h2 className="text-4xl leading-tight font-bold tracking-tight italic sm:text-5xl">
              {s("results.ctaTitle")} <span className="o-hl">{s("results.ctaHl")}</span>
            </h2>
            <p className="mt-4 text-lg text-muted-foreground">{s("results.ctaBody")}</p>
            <div className="mt-8 [&>div]:justify-start">{cta}</div>
          </div>
        </div>
      </div>
    </section>
  );
}
