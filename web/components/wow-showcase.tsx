"use client";

import { Check, GitPullRequest, Globe, MessageSquare, Smartphone, Sparkles, Trophy, X } from "lucide-react";
import { type ReactNode, useEffect, useState, useSyncExternalStore } from "react";
import { WinnerBoard } from "@/components/landing-visuals";

// The landing's "wow" moments, each a short looping scene. Scene content is sample product data
// (repo names, chat lines, app screens), so it is not in the UI catalogs; titles and benefits are.
// Without motion every scene shows its final state and nothing advances on its own.

export type WowItem = { id: "minutes" | "ai" | "mobile" | "support" | "words" | "live"; title: string; gain: string };

const ICON = { minutes: GitPullRequest, ai: Sparkles, mobile: Smartphone, support: MessageSquare, words: Trophy, live: Globe };
const ACC = { minutes: "acc-green", ai: "acc-violet", mobile: "acc-coral", support: "acc-cyan", words: "acc-amber", live: "acc-pink" };
const DWELL = 8000;

const RM = "(prefers-reduced-motion: reduce)";
function useReduced() {
  return useSyncExternalStore(
    (f) => (window.matchMedia(RM).addEventListener("change", f), () => window.matchMedia(RM).removeEventListener("change", f)),
    () => window.matchMedia(RM).matches,
    () => false,
  );
}

/** Characters of `text` revealed over time (all of it with reduced motion). */
function useType(text: string, start: number, cps = 28) {
  const reduced = useReduced();
  const [n, setN] = useState(0);
  // Scenes remount when picked, so the count always starts at 0.
  useEffect(() => {
    if (reduced) return;
    let id: ReturnType<typeof setInterval> | undefined;
    const t = setTimeout(() => {
      id = setInterval(() => setN((x) => (x >= text.length ? (clearInterval(id), x) : x + 1)), 1000 / cps);
    }, start);
    return () => (clearTimeout(t), clearInterval(id));
  }, [text, start, cps, reduced]);
  return reduced ? text : text.slice(0, n);
}

const Frame = ({ children, bar }: { children: ReactNode; bar?: ReactNode }) => (
  <div className="overflow-hidden rounded-xl border bg-background shadow-lg">
    <div className="flex items-center gap-1.5 border-b bg-muted/50 px-3 py-2">
      <span className="size-2.5 rounded-full bg-[#ff5f57]" />
      <span className="size-2.5 rounded-full bg-[#febc2e]" />
      <span className="size-2.5 rounded-full bg-[#28c840]" />
      {bar && <div className="ms-3 min-w-0 flex-1 truncate rounded-md bg-background px-2 py-0.5 font-mono text-[11px] text-muted-foreground">{bar}</div>}
    </div>
    {children}
  </div>
);

const FILES = [["fr", "412"], ["de", "412"], ["es", "412"], ["ja", "412"], ["ar", "412"], ["pt", "412"]];

function ScenePR() {
  return (
    <Frame bar="github.com/acme/web/pull/128">
      <div className="p-4 sm:p-5">
        <div className="flex items-center gap-2">
          <span className="wow-in inline-flex items-center gap-1 rounded-full bg-emerald-600 px-2 py-0.5 text-[11px] font-medium text-white">
            <GitPullRequest className="size-3" /> Open
          </span>
          <span className="font-mono text-[11px] text-muted-foreground">glot/translate → main</span>
        </div>
        <p className="wow-in mt-2 font-semibold">Translate the app into 6 languages</p>
        <ul className="mt-4 space-y-1.5 font-mono text-xs">
          {FILES.map(([l, n], i) => (
            <li key={l} className="wow-in flex items-center justify-between rounded-md border px-2.5 py-1.5" style={{ animationDelay: `${600 + i * 450}ms` }}>
              <span>messages/{l}.json</span>
              <span className="flex items-center gap-2">
                <span className="text-emerald-600 dark:text-emerald-400">+{n}</span>
                <Check className="size-3.5 text-emerald-600 dark:text-emerald-400" />
              </span>
            </li>
          ))}
        </ul>
        <div className="mt-4 flex items-center justify-between gap-3">
          <span className="wow-in text-xs text-muted-foreground" style={{ animationDelay: "3400ms" }}>2,472 sentences · 0 broken</span>
          <span className="wow-pop rounded-md bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white" style={{ animationDelay: "3700ms" }}>Ready to merge</span>
        </div>
      </div>
    </Frame>
  );
}

const ANSWER = "ご注文は明日の午前中にお届けします。追跡番号はメールでお送りしました。ほかにお手伝いできることはありますか？";

function SceneAI() {
  const typed = useType(ANSWER, 1200, 22);
  return (
    <Frame bar="acme.com · support chat">
      <div className="space-y-3 p-4 sm:p-5" lang="ja">
        <p className="wow-in ms-auto w-fit max-w-[80%] rounded-2xl rounded-ee-sm bg-primary px-3.5 py-2 text-sm text-primary-foreground">注文はいつ届きますか？</p>
        <div className="wow-in max-w-[85%]" style={{ animationDelay: "700ms" }}>
          <p className="min-h-[4.5rem] rounded-2xl rounded-es-sm bg-muted px-3.5 py-2 text-sm leading-relaxed">
            {typed}
            <span className="wow-caret">▍</span>
          </p>
          <p className="mt-1.5 inline-flex items-center gap-1 text-[11px] text-muted-foreground" lang="en">
            <Sparkles className="acc-ink size-3" /> AI answer in English · translated to Japanese as it streams
          </p>
        </div>
      </div>
    </Frame>
  );
}

const PAY = ["Pay now", "Payer maintenant", "Jetzt bezahlen", "今すぐ支払う", "Pagar ahora"];

function SceneMobile() {
  const reduced = useReduced();
  const [i, setI] = useState(0);
  useEffect(() => {
    if (reduced) return;
    const id = setInterval(() => setI((x) => (x + 1) % PAY.length), 1400);
    return () => clearInterval(id);
  }, [reduced]);
  return (
    <div className="flex items-center justify-center gap-6">
      <div className="relative w-56 rounded-[2.2rem] border-[6px] border-foreground/85 bg-background p-3 shadow-xl">
        <div className="mx-auto mb-3 h-4 w-20 rounded-full bg-foreground/85" />
        <div className="wow-drop absolute inset-x-4 top-9 z-10 flex items-center gap-2 rounded-xl border bg-card/95 p-2 shadow-md backdrop-blur" style={{ animationDelay: "500ms" }}>
          <span className="acc-coral tile size-7 shrink-0 rounded-lg text-[10px] font-bold">g</span>
          <span className="text-[11px] leading-tight">
            <strong className="block">glot</strong>3 texts updated · no store release
          </span>
        </div>
        <div className="space-y-2 px-1 pt-8 pb-2">
          <div className="h-24 rounded-xl bg-gradient-to-br from-orange-300 to-pink-400" />
          <div className="h-2.5 w-3/4 rounded-full bg-muted" />
          <div className="h-2.5 w-1/2 rounded-full bg-muted" />
          <p className="pt-1 text-lg font-semibold">€49.00</p>
          <p key={i} className="wow-swap rounded-xl bg-foreground py-2.5 text-center text-sm font-semibold text-background">
            {PAY[i]}
          </p>
        </div>
      </div>
    </div>
  );
}

function Bubble({ who, lang, text, note, delay, mine }: { who: string; lang: string; text: string; note?: string; delay: number; mine?: boolean }) {
  return (
    <div className={`wow-in ${mine ? "ms-auto text-end" : ""} max-w-[92%]`} style={{ animationDelay: `${delay}ms` }}>
      <p className="mb-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">{who}</p>
      <p lang={lang} className={`inline-block rounded-2xl px-3 py-2 text-start text-sm ${mine ? "bg-primary text-primary-foreground" : "bg-muted"}`}>{text}</p>
      {note && <p className="mt-1 text-[10px] text-muted-foreground">{note}</p>}
    </div>
  );
}

function SceneSupport() {
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <Frame bar="Customer · 日本語">
        <div className="space-y-3 p-3.5">
          <Bubble who="Yuki" lang="ja" text="返金はできますか？サイズが合いませんでした。" delay={200} />
          <Bubble who="Acme" lang="ja" text="もちろんです。返品ラベルをメールでお送りしました。" delay={3400} mine />
        </div>
      </Frame>
      <Frame bar="Your team · Français">
        <div className="space-y-3 p-3.5">
          <Bubble who="Yuki" lang="fr" text="Puis-je être remboursée ? La taille ne convenait pas." note="Traduit du japonais" delay={1100} />
          <Bubble who="Vous" lang="fr" text="Bien sûr. Je vous ai envoyé l'étiquette de retour par e-mail." delay={2300} mine />
        </div>
      </Frame>
    </div>
  );
}

function SceneLive() {
  return (
    <Frame bar={
      <span>
        acme.com/?lang=<span className="wow-strike">zh-CN</span>
        <span className="wow-in font-semibold text-emerald-600 dark:text-emerald-400" style={{ animationDelay: "2600ms" }}>zh</span>
      </span>
    }>
      <div className="grid gap-4 p-4 sm:grid-cols-[1fr_auto] sm:p-5">
        <div className="relative min-h-24">
          <p className="wow-out text-2xl font-semibold" style={{ animationDelay: "3000ms" }}>Welcome to Acme</p>
          <p lang="zh" className="wow-in absolute inset-0 text-2xl font-semibold" style={{ animationDelay: "3000ms" }}>欢迎来到 Acme</p>
          <div className="mt-12 space-y-2">
            <div className="h-2.5 w-4/5 rounded-full bg-muted" />
            <div className="h-2.5 w-3/5 rounded-full bg-muted" />
          </div>
        </div>
        <ul className="space-y-2 text-xs sm:w-52">
          <li className="wow-in flex items-start gap-2 rounded-lg border border-red-500/30 bg-red-500/5 p-2" style={{ animationDelay: "600ms" }}>
            <X className="mt-0.5 size-3.5 shrink-0 text-red-500" />
            <span>Your site ignores <code>zh-CN</code></span>
          </li>
          <li className="wow-in flex items-start gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-2" style={{ animationDelay: "1700ms" }}>
            <Check className="mt-0.5 size-3.5 shrink-0 text-emerald-600" />
            <span>It accepts <code>zh</code>: glot answers with that code</span>
          </li>
          <li className="wow-pop flex items-center gap-2 rounded-lg bg-emerald-600 p-2 font-medium text-white" style={{ animationDelay: "3200ms" }}>
            <Check className="size-3.5" /> Chinese is live
          </li>
        </ul>
      </div>
    </Frame>
  );
}

const SCENES = { minutes: ScenePR, ai: SceneAI, mobile: SceneMobile, support: SceneSupport, words: WinnerBoard, live: SceneLive };

export function WowShowcase({ items, label }: { items: WowItem[]; label: string }) {
  const [active, setActive] = useState(0);
  const [round, setRound] = useState(0); // replays a scene when it is picked again
  const pick = (i: number) => (setActive(i), setRound((r) => r + 1));
  const item = items[active];
  const Scene = SCENES[item.id];
  return (
    <div className="wow grid gap-6 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)] lg:gap-10">
      <div role="tablist" aria-label={label} aria-orientation="vertical" className="flex gap-2 overflow-x-auto pb-1 lg:flex-col lg:overflow-visible">
        {items.map((it, i) => {
          const Icon = ICON[it.id];
          const on = i === active;
          return (
            <button
              key={it.id}
              type="button"
              role="tab"
              id={`wow-tab-${it.id}`}
              aria-selected={on}
              aria-controls="wow-panel"
              onClick={() => pick(i)}
              className={`${ACC[it.id]} relative shrink-0 overflow-hidden rounded-xl border p-3 text-start transition-colors duration-200 lg:p-4 ${on ? "border-[color-mix(in_oklch,var(--acc)_45%,var(--border))] bg-card shadow-sm" : "border-transparent hover:bg-card/60"}`}
            >
              <span className="flex items-center gap-3">
                <span className="tile size-9 shrink-0">
                  <Icon aria-hidden className="size-4.5" />
                </span>
                <span className="font-semibold whitespace-nowrap lg:whitespace-normal">{it.title}</span>
              </span>
              <span className={`mt-2 hidden text-sm leading-relaxed text-muted-foreground ${on ? "lg:block" : ""}`}>{it.gain}</span>
              {on && (
                <span
                  key={round}
                  aria-hidden
                  className="wow-progress absolute inset-x-0 bottom-0 h-0.5"
                  style={{ background: "var(--acc)", animationDuration: `${DWELL}ms` }}
                  onAnimationEnd={() => pick((active + 1) % items.length)}
                />
              )}
            </button>
          );
        })}
      </div>
      <div id="wow-panel" role="tabpanel" aria-labelledby={`wow-tab-${item.id}`} className={`${ACC[item.id]} relative`}>
        <div aria-hidden className="wow-halo pointer-events-none absolute -inset-6 -z-10 rounded-[2rem]" />
        <p className="mb-4 text-sm text-muted-foreground lg:hidden">{item.gain}</p>
        <div key={`${item.id}${round}`} aria-hidden className="wow-stage flex min-h-[22rem] items-center">
          <div className="w-full">
            <Scene />
          </div>
        </div>
      </div>
    </div>
  );
}
