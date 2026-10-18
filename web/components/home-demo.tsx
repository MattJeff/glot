"use client";

// The home page demo: one key → three AI versions → checks per locale → audiences → result → pull request.
// Everything shown is example data (labelled as such); refusal messages are glot's real wording (crates/core).
// Motion is CSS only. Autoplay advances on the progress bar's animationend, so it pauses for free, and it never
// starts with prefers-reduced-motion: every step is then a static, complete picture you step through.
import { Check, CircleCheck, CircleX, Clock, GitMerge, GitPullRequest, Minus, Pause, Play, RotateCcw, RotateCw } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

const KEY = "home.hero.title";
const LOCALES = ["en", "de", "ja", "ar"] as const;
type L = (typeof LOCALES)[number];
const OFFICIAL: Record<L, string> = { en: "Send your invoices in minutes.", de: "Rechnungen in Minuten versenden.", ja: "請求書を数分で送信。", ar: "أرسل فواتيرك في دقائق." };
type Intent = "simplicity" | "benefit" | "urgency";
const VERSIONS: { id: Intent; text: Record<L, string> | { en: string }; refused?: string; retry?: { locale: L; msg: string } }[] = [
  { id: "simplicity", text: { en: "Invoices in a few clicks.", de: "Rechnungen mit wenigen Klicks.", ja: "請求書は数クリックで。", ar: "فواتيرك ببضع نقرات." } },
  {
    id: "benefit",
    text: { en: "Get paid faster. Send invoices in minutes.", de: "Schneller bezahlt. Rechnungen in Minuten.", ja: "もっと早く入金。請求書は数分で。", ar: "احصل على أموالك أسرع. فواتيرك في دقائق." },
    retry: { locale: "de", msg: "Too wide for its box in the app: one line of 480px at Geist 40px/600; it renders 702px wide." },
  },
  {
    id: "urgency",
    text: { en: "Today only: send unlimited invoices." },
    refused: `"Today only": urgency or scarcity with no source. Declare the real deadline or stock as a fact (e.g. "offer ends 31 Oct") or remove the wording.`,
  },
];
// Traffic share per audience: official, simplicity, benefit (urgency never went live).
const AUDIENCES = [
  { id: "de", share: [18, 64, 18], visitors: 4812 },
  { id: "fr", share: [34, 30, 36], visitors: 2240 },
  { id: "rest", share: [27, 49, 24], visitors: 9530 },
] as const;
const SERIES = ["official", "simplicity", "benefit"] as const;
const BAR = ["bg-muted-foreground/45", "bg-brand", "bg-info"];
const STEPS = ["write", "check", "serve", "learn", "ship"] as const;
const STEP_MS = 5200;
// CLI-style output lines (product data, shown verbatim like the terminal).
const OUT = { width: "de · width", urgency: "en · fake_urgency", retryOk: "→ retry passed", promote: "promote:" };
const DAY = 14;

const dirOf = (l: string) => (l === "ar" ? "rtl" : "ltr");

/** Count up to `to` once `run` is true (instant with reduced motion). */
function useCount(to: number, run: boolean, ms = 900) {
  const [n, setN] = useState(0);
  useEffect(() => {
    if (!run) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      const id = requestAnimationFrame(() => setN(to));
      return () => cancelAnimationFrame(id);
    }
    let raf = 0;
    const t0 = performance.now();
    const tick = (now: number) => {
      const p = Math.min(1, (now - t0) / ms);
      setN(Math.round(to * (1 - (1 - p) ** 3)));
      if (p < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [to, run, ms]);
  return n;
}

export function HomeDemo() {
  const t = useTranslations("home.demo");
  const root = useRef<HTMLDivElement>(null);
  const [step, setStep] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [visible, setVisible] = useState(false);
  const [announce, setAnnounce] = useState("");

  // Autoplay only on screen and without reduced motion.
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const io = new IntersectionObserver(([e]) => {
      setVisible(e.isIntersecting);
      if (e.isIntersecting && !reduce) setPlaying((p) => p || !el.dataset.touched);
    }, { threshold: 0.35 });
    io.observe(el);
    return () => io.disconnect();
  }, []);

  const go = (i: number, user = true) => {
    setStep(i);
    if (user) {
      root.current?.setAttribute("data-touched", "1");
      setPlaying(false);
      setAnnounce(t("announce", { n: i + 1, total: STEPS.length, title: t(`${STEPS[i]}.title`) }));
    }
  };
  const next = () => (step < STEPS.length - 1 ? go(step + 1, false) : setPlaying(false));
  const running = playing && visible;
  const done = step === STEPS.length - 1 && !playing;

  return (
    <div ref={root} className="demo reveal overflow-hidden rounded-2xl border bg-card shadow-lg" role="group" aria-label={t("label")}>
      {/* Key bar: what is under test, always visible. */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b bg-muted/40 px-4 py-3 sm:px-5">
        <span className="text-xs font-medium text-muted-foreground">{t("key")}</span>
        <code dir="ltr" className="rounded-md border bg-background px-2 py-0.5 font-mono text-xs">
          {KEY}
        </code>
        <span className="min-w-0 truncate text-sm" title={OFFICIAL.en}>
          <span className="text-muted-foreground">{t("official")}: </span>
          <span lang="en">“{OFFICIAL.en}”</span>
        </span>
        <span className="ms-auto rounded-full border border-dashed px-2 py-0.5 text-[11px] font-medium text-muted-foreground">{t("example")}</span>
      </div>

      <div className="grid lg:grid-cols-[15rem_minmax(0,1fr)]">
        {/* Steps: a readable summary of the whole flow on large screens, compact tabs on phones. */}
        <div className="border-b p-2 lg:border-e lg:border-b-0 lg:p-3">
          <ol aria-label={t("steps")} className="grid grid-cols-5 gap-1 lg:grid-cols-1">
            {STEPS.map((s, i) => {
              const active = i === step;
              const past = i < step;
              return (
                <li key={s}>
                  <button
                    type="button"
                    onClick={() => go(i)}
                    aria-current={active ? "step" : undefined}
                    className={cn(
                      "relative flex min-h-11 w-full flex-col items-center gap-1 overflow-hidden rounded-lg px-1 py-1.5 text-center transition-colors duration-150 hover:bg-muted lg:flex-row lg:items-start lg:gap-3 lg:px-3 lg:py-2.5 lg:text-start",
                      active && "bg-muted",
                    )}
                  >
                    <span
                      className={cn(
                        "grid size-6 shrink-0 place-items-center rounded-full border font-mono text-[11px] tabular-nums transition-colors duration-200",
                        active && "border-brand bg-brand text-brand-foreground",
                        past && "border-brand/50 text-brand",
                      )}
                    >
                      {past ? <Check aria-hidden className="size-3.5" /> : i + 1}
                    </span>
                    <span className="min-w-0">
                      <span className="block text-xs font-medium lg:hidden">{t(`${s}.label`)}</span>
                      <span className="hidden text-sm font-medium lg:block">{t(`${s}.title`)}</span>
                      <span className="mt-0.5 hidden text-xs leading-relaxed text-muted-foreground lg:block">{t(`${s}.body`)}</span>
                    </span>
                    {active && running && (
                      <span
                        aria-hidden
                        key={`bar-${step}`}
                        onAnimationEnd={next}
                        className="demo-progress absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-brand"
                        style={{ animationDuration: `${STEP_MS}ms` }}
                      />
                    )}
                  </button>
                </li>
              );
            })}
          </ol>
        </div>

        <div className="flex min-w-0 flex-col p-4 sm:p-5">
          <div className="mb-3 lg:hidden">
            <p className="text-sm font-medium">{t(`${STEPS[step]}.title`)}</p>
            <p className="text-xs text-muted-foreground">{t(`${STEPS[step]}.body`)}</p>
          </div>
          <div key={step} className="demo-stage min-h-[23rem] flex-1 sm:min-h-[21rem]">
            {step === 0 && <Write />}
            {step === 1 && <Checks />}
            {step === 2 && <Serve />}
            {step === 3 && <Learn onPromote={() => go(4)} />}
            {step === 4 && <Ship />}
          </div>
          <div className="mt-4 flex items-center gap-2 border-t pt-3">
            <button
              type="button"
              onClick={() => {
                if (done) {
                  setStep(0);
                  setPlaying(true);
                } else setPlaying((p) => !p);
                root.current?.setAttribute("data-touched", "1");
              }}
              className="inline-flex min-h-9 items-center gap-1.5 rounded-md border px-3 text-sm font-medium transition-colors hover:bg-muted"
            >
              {done ? <RotateCcw aria-hidden className="size-3.5" /> : playing ? <Pause aria-hidden className="size-3.5" /> : <Play aria-hidden className="size-3.5" />}
              {done ? t("replay") : playing ? t("pause") : t("play")}
            </button>
            <p className="ms-auto text-xs text-muted-foreground tabular-nums">
              {step + 1}/{STEPS.length}
            </p>
          </div>
        </div>
      </div>
      <p role="status" aria-live="polite" className="sr-only">
        {announce}
      </p>
    </div>
  );
}

function IntentPill({ id }: { id: Intent }) {
  const t = useTranslations("home.demo.intent");
  return <span className="rounded-full border bg-background px-2 py-0.5 text-[11px] font-medium text-muted-foreground">{t(id)}</span>;
}

function Write() {
  const t = useTranslations("home.demo");
  return (
    <ul className="grid grid-cols-[minmax(0,1fr)] gap-3">
      <li className="flex min-w-0 items-center gap-2 rounded-xl border border-dashed px-3.5 py-2.5 text-sm text-muted-foreground">
        <span aria-hidden className="grid size-6 place-items-center rounded-md bg-muted font-mono text-[11px] font-semibold">
          {"A"}
        </span>
        <span className="shrink-0 text-[11px] font-medium whitespace-nowrap">{t("official")}</span>
        <span lang="en" className="min-w-0 truncate" title={OFFICIAL.en}>“{OFFICIAL.en}”</span>
      </li>
      {VERSIONS.map((v, i) => (
        <li key={v.id} className="demo-in rounded-xl border bg-background p-3.5" style={{ animationDelay: `${150 + i * 380}ms` }}>
          <div className="flex items-center gap-2">
            <span className="grid size-6 place-items-center rounded-md bg-muted font-mono text-[11px] font-semibold" aria-hidden>
              {String.fromCharCode(66 + i)}
            </span>
            <IntentPill id={v.id} />
            <span className="demo-status ms-auto text-[11px]" style={{ ["--d" as string]: `${900 + i * 380}ms` }}>
              <span aria-hidden className="demo-writing text-muted-foreground">{t("writing")}</span>
              <span className="demo-ready inline-flex items-center gap-1 font-medium text-success-foreground">
                <Check aria-hidden className="size-3" /> {t("ready")}
              </span>
            </span>
          </div>
          <p lang="en" className="demo-type mt-2 text-base font-medium sm:text-lg" style={{ animationDelay: `${300 + i * 380}ms` }}>
            “{v.text.en}”
          </p>
        </li>
      ))}
    </ul>
  );
}

function Checks() {
  const t = useTranslations("home.demo");
  // Cells revealed one by one; the urgency version stops at the source (en), nothing else is translated.
  const cells = VERSIONS.flatMap((v) => (v.refused ? [`${v.id}-en`] : LOCALES.map((l) => `${v.id}-${l}`)));
  const total = cells.length;
  const passed = useCount(total - 1, true, 1300);
  const order = (id: string) => cells.indexOf(id);
  return (
    <div>
      <div className="overflow-x-auto rounded-xl border bg-background">
        <table className="w-full min-w-[19rem] text-sm">
          <caption className="sr-only">{t("check.title")}</caption>
          <thead>
            <tr className="border-b text-xs text-muted-foreground">
              <th scope="col" className="px-3 py-2 text-start font-medium">{t("version")}</th>
              {LOCALES.map((l) => (
                <th key={l} scope="col" className="px-2 py-2 text-center font-mono font-medium">
                  {l}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {VERSIONS.map((v) => (
              <tr key={v.id} className="border-b last:border-0">
                <th scope="row" className="px-3 py-2.5 text-start font-normal">
                  <IntentPill id={v.id} />
                </th>
                {LOCALES.map((l) => {
                  const id = `${v.id}-${l}`;
                  const i = order(id);
                  if (i < 0)
                    return (
                      <td key={l} className="px-2 py-2.5 text-center text-muted-foreground">
                        <Minus aria-hidden className="mx-auto size-3.5 opacity-50" />
                        <span className="sr-only">—</span>
                      </td>
                    );
                  const bad = !!v.refused;
                  const retried = v.retry?.locale === l;
                  return (
                    <td key={l} className="px-2 py-2.5 text-center">
                      <span className="demo-pop inline-grid size-7 place-items-center rounded-full" style={{ animationDelay: `${200 + i * 140}ms` }}>
                        {bad ? (
                          <CircleX role="img" aria-label={t("fail")} className="size-5 text-destructive" />
                        ) : retried ? (
                          <span className="relative">
                            <RotateCw role="img" aria-label={t("retried")} className="size-5 text-warning-foreground" />
                          </span>
                        ) : (
                          <CircleCheck role="img" aria-label={t("pass")} className="size-5 text-success-foreground" />
                        )}
                      </span>
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-3 flex items-center gap-2 text-sm font-medium tabular-nums">
        <CircleCheck aria-hidden className="size-4 text-success-foreground" />
        <span aria-hidden>{t("checks", { passed, total })}</span>
        <span className="sr-only">{t("checks", { passed: total - 1, total })}</span>
      </p>
      <div dir="ltr" lang="en" className="mt-3 space-y-2 rounded-xl border term p-3 font-mono text-[11.5px] leading-relaxed">
        {VERSIONS.filter((v) => v.retry).map((v) => (
          <p key={v.id} className="demo-in flex gap-1.5 term-warn" style={{ animationDelay: `${200 + order(`${v.id}-de`) * 140}ms` }}>
            <RotateCw aria-hidden className="mt-0.5 size-3.5 shrink-0" />
            <span>
              <span className="term-faint">{OUT.width}</span> {v.retry!.msg} <span className="term-ok">{OUT.retryOk}</span>
            </span>
          </p>
        ))}
        {VERSIONS.filter((v) => v.refused).map((v) => (
          <p key={v.id} className="demo-in flex gap-1.5 term-err" style={{ animationDelay: `${300 + order(`${v.id}-en`) * 140}ms` }}>
            <CircleX aria-hidden className="mt-0.5 size-3.5 shrink-0" />
            <span>
              <span className="sr-only">{t("refused")}: </span>
              <span className="term-faint">{OUT.urgency}</span> {v.refused}
            </span>
          </p>
        ))}
      </div>
    </div>
  );
}

function Serve() {
  const t = useTranslations("home.demo");
  const ti = useTranslations("home.demo.intent");
  const [settled, setSettled] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => setSettled(true), 250);
    return () => clearTimeout(id);
  }, []);
  const day = useCount(DAY, true, 2200);
  const label = (s: (typeof SERIES)[number]) => (s === "official" ? t("official") : ti(s));
  return (
    <div>
      <div className="mb-3 flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span>{t("traffic")}</span>
        <span className="inline-flex items-center gap-1 tabular-nums">
          <Clock aria-hidden className="size-3.5" /> {t("day", { day: Math.max(day, 1) })}
        </span>
      </div>
      <ul className="space-y-4">
        {AUDIENCES.map((a, i) => (
          <Audience key={a.id} a={a} i={i} settled={settled} label={label} />
        ))}
      </ul>
      <ul aria-hidden className="mt-4 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {SERIES.map((s, i) => (
          <li key={s} className="inline-flex items-center gap-1.5">
            <span className={cn("size-2.5 rounded-sm", BAR[i])} /> {label(s)}
          </li>
        ))}
      </ul>
    </div>
  );
}

function Audience({ a, i, settled, label }: { a: (typeof AUDIENCES)[number]; i: number; settled: boolean; label: (s: (typeof SERIES)[number]) => string }) {
  const t = useTranslations("home.demo");
  const visitors = useCount(a.visitors, true, 2200);
  return (
    <li className="demo-in" style={{ animationDelay: `${i * 120}ms` }}>
      <div className="mb-1.5 flex items-baseline justify-between gap-2 text-sm">
        <span className="font-medium">{t(`audience.${a.id}`)}</span>
        <span className="text-xs text-muted-foreground tabular-nums">
          <span aria-hidden>{t("visitors", { count: visitors })}</span>
          <span className="sr-only">{t("visitors", { count: a.visitors })}</span>
        </span>
      </div>
      <div aria-hidden className="flex h-3 gap-0.5 overflow-hidden rounded-full">
        {a.share.map((v, j) => (
          <span key={j} className={cn("demo-share h-full", BAR[j])} style={{ width: `${settled ? v : 100 / 3}%`, transitionDelay: `${i * 120}ms` }} />
        ))}
      </div>
      <p className="mt-1 text-[11px] text-muted-foreground tabular-nums">{a.share.map((v, j) => `${label(SERIES[j])} ${v}%`).join(" · ")}</p>
    </li>
  );
}

function Learn({ onPromote }: { onPromote: () => void }) {
  const t = useTranslations("home.demo");
  const ti = useTranslations("home.demo.intent");
  const rows = [
    { id: "de", kind: "win", p: 97, args: { variant: ti("simplicity"), lift: "+18%", low: "+6%", high: "+30%", probability: "97%" } },
    { id: "fr", kind: "none" },
    { id: "rest", kind: "win", p: 96, args: { variant: ti("simplicity"), lift: "+9%", low: "+2%", high: "+16%", probability: "96%" } },
  ] as const;
  return (
    <ul className="space-y-3">
      {rows.map((r, i) => (
        <li key={r.id} className="demo-in flex gap-3 rounded-xl border bg-background p-3.5" style={{ animationDelay: `${i * 260}ms` }}>
          {r.kind === "win" ? <Gauge value={r.p} delay={i * 260} /> : <span className="grid size-12 shrink-0 place-items-center rounded-full border border-dashed text-muted-foreground"><Minus aria-hidden className="size-4" /></span>}
          <div className="min-w-0">
            <p className="text-xs font-medium text-muted-foreground">{t(`audience.${r.id}`)}</p>
            <p className="mt-0.5 text-sm leading-relaxed text-pretty">{r.kind === "win" ? t("win", r.args) : t("none")}</p>
          </div>
        </li>
      ))}
      <li className="demo-in flex justify-end" style={{ animationDelay: "800ms" }}>
        <button
          type="button"
          onClick={onPromote}
          className="inline-flex min-h-9 items-center gap-1.5 rounded-md bg-brand px-3 text-sm font-medium text-brand-foreground transition-[transform,opacity] duration-150 hover:opacity-90 active:scale-[0.98] motion-reduce:active:scale-100"
        >
          <GitPullRequest aria-hidden className="size-4" /> {t("promote")}
        </button>
      </li>
    </ul>
  );
}

/** Probability ring: decorative, the sentence next to it carries the number. */
function Gauge({ value, delay }: { value: number; delay: number }) {
  const r = 20;
  const c = 2 * Math.PI * r;
  const n = useCount(value, true, 1200);
  return (
    <span aria-hidden className="relative grid size-12 shrink-0 place-items-center">
      <svg viewBox="0 0 48 48" className="absolute inset-0 -rotate-90">
        <circle cx="24" cy="24" r={r} fill="none" strokeWidth="4" className="stroke-muted" />
        <circle
          cx="24"
          cy="24"
          r={r}
          fill="none"
          strokeWidth="4"
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - value / 100)}
          className="demo-ring stroke-brand"
          style={{ ["--c" as string]: c, animationDelay: `${delay}ms` }}
        />
      </svg>
      <span className="text-[11px] font-semibold tabular-nums">{n}%</span>
    </span>
  );
}

function Ship() {
  const t = useTranslations("home.demo");
  const ti = useTranslations("home.demo.intent");
  const [merged, setMerged] = useState(false);
  const win = VERSIONS[0].text as Record<L, string>;
  return (
    <div className="demo-in rounded-xl border bg-background">
      <div className="flex flex-wrap items-center gap-2 border-b px-3.5 py-3">
        {merged ? <GitMerge aria-hidden className="size-4 text-info-foreground" /> : <GitPullRequest aria-hidden className="size-4 text-success-foreground" />}
        <p dir="ltr" className="min-w-0 font-mono text-xs font-medium">
          {`${OUT.promote} ${KEY} → ${ti("simplicity")} (${LOCALES.join(", ")})`}
        </p>
        <span className="ms-auto text-xs text-muted-foreground">{t("files", { count: LOCALES.length })}</span>
      </div>
      <div dir="ltr" className="space-y-2 p-3.5 font-mono text-[11.5px] leading-relaxed">
        {LOCALES.map((l, i) => (
          <div key={l} className="demo-in overflow-hidden rounded-md border" style={{ animationDelay: `${120 + i * 160}ms` }}>
            <p className="border-b bg-muted/50 px-2.5 py-1 text-muted-foreground">messages/{l}.json</p>
            <p className="truncate bg-destructive/8 px-2.5 py-0.5" title={OFFICIAL[l]}>
              <span aria-hidden className="select-none">− </span>&quot;<bdi lang={l} dir={dirOf(l)}>{OFFICIAL[l]}</bdi>&quot;
            </p>
            <p className="truncate bg-success/10 px-2.5 py-0.5" title={win[l]}>
              <span aria-hidden className="select-none">+ </span>&quot;<bdi lang={l} dir={dirOf(l)}>{win[l]}</bdi>&quot;
            </p>
          </div>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-3 border-t px-3.5 py-3">
        <button
          type="button"
          onClick={() => setMerged(true)}
          disabled={merged}
          className="inline-flex min-h-9 items-center gap-1.5 rounded-md bg-brand px-3 text-sm font-medium text-brand-foreground transition-[transform,opacity] duration-150 hover:opacity-90 active:scale-[0.98] disabled:opacity-60 motion-reduce:active:scale-100"
        >
          <GitMerge aria-hidden className="size-4" /> {t("merge")}
        </button>
        <p role="status" className="text-sm">
          {merged && (
            <span className="demo-pop inline-flex items-center gap-1.5 font-medium text-success-foreground">
              <CircleCheck aria-hidden className="size-4" /> {t("merged")}
            </span>
          )}
        </p>
      </div>
    </div>
  );
}
