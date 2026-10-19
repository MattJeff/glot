"use client";

import { Check, CircleCheck, CircleX, Copy, Pause, Play, RotateCcw, TriangleAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/** The primary CTA: one command, copied in one click, announced to screen readers. */
export function CopyCommand({ command, className }: { command: string; className?: string }) {
  const t = useTranslations("landing.hero");
  const [done, setDone] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setDone(true);
      setTimeout(() => setDone(false), 1600);
    } catch {
      // Clipboard blocked: the command stays selectable on screen.
    }
  };
  return (
    <button
      type="button"
      onClick={copy}
      dir="ltr"
      className={cn(
        "group inline-flex h-11 items-center gap-3 rounded-lg border bg-foreground ps-4 pe-2 font-mono text-sm text-background shadow-sm transition-[transform,box-shadow] duration-150 ease-out hover:shadow-md active:scale-[0.98] motion-reduce:active:scale-100",
        className,
      )}
    >
      <span aria-hidden className="opacity-50 select-none">$</span>
      <span className="select-all">{command}</span>
      <span className="grid size-7 place-items-center rounded-md bg-background/15 transition-colors group-hover:bg-background/25">
        {done ? <Check aria-hidden className="size-4" /> : <Copy aria-hidden className="size-4" />}
      </span>
      <span className="sr-only">{t("copy")}</span>
      <span role="status" className="sr-only">
        {done ? t("copied") : ""}
      </span>
    </button>
  );
}

/** Hero recording: poster first (no layout shift), autoplay only without reduced motion, always pausable. */
export function DemoVideo({ label }: { label: string }) {
  const t = useTranslations("landing.hero");
  const ref = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);

  useEffect(() => {
    const v = ref.current;
    if (!v || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    v.play().catch(() => {});
  }, []);

  const toggle = () => {
    const v = ref.current;
    if (!v) return;
    if (v.paused) v.play().catch(() => {});
    else v.pause();
  };

  return (
    <div dir="ltr" className="group relative overflow-hidden rounded-xl border term shadow-lg">
      <video
        ref={ref}
        poster="/launch/refusal.webp"
        muted
        loop
        playsInline
        preload="metadata"
        aria-label={label}
        width={1100}
        height={680}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        className="aspect-[1100/680] h-auto w-full"
      >
        <source src="/launch/refusal.mp4" type="video/mp4" />
        <source src="/launch/refusal.webm" type="video/webm" />
      </video>
      <button
        type="button"
        onClick={toggle}
        aria-label={playing ? t("pause") : t("play")}
        className="absolute end-3 bottom-3 grid size-10 place-items-center rounded-full border border-white/15 bg-black/60 text-white backdrop-blur transition-opacity duration-150 hover:bg-black/80 focus-visible:opacity-100 sm:opacity-0 sm:group-hover:opacity-100 [@media(hover:none)]:opacity-100"
      >
        {playing ? <Pause aria-hidden className="size-4" /> : <Play aria-hidden className="size-4" />}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------- playground (glot-core → WASM)

type Issue = { code: string; severity: "error" | "warning"; message: string };
type Validate = (input: { source: string; target: string; locale: string }) => Issue[];

/** Same raw C ABI as @glot-i18n/runtime's loadValidator (crates/wasm): alloc → check → [len u32][JSON]. */
async function loadValidator(): Promise<Validate> {
  const { instance } = await WebAssembly.instantiateStreaming(fetch("/glot.wasm"), {});
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const x = instance.exports as any;
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  return (input) => {
    const data = enc.encode(JSON.stringify(input));
    const p = x.alloc(data.length);
    new Uint8Array(x.memory.buffer, p, data.length).set(data);
    const r = x.check(p, data.length);
    x.dealloc(p, data.length);
    const len = new DataView(x.memory.buffer).getUint32(r, true);
    const out = dec.decode(new Uint8Array(x.memory.buffer, r + 4, len));
    x.dealloc(r, len + 4);
    return JSON.parse(out);
  };
}

const PLURAL_EN = "{count, plural, one {# unread message} other {# unread messages}}";
// Challenges are product data (ICU messages in their own languages), not UI copy.
function useChallenges() {
  const t = useTranslations("landing.hero");
  return [
  { id: "placeholder", tag: "{name}", source: t("greeting.welcome_back"), locale: "fr", target: "Bon retour, {nom} !" },
  {
    id: "plural",
    tag: "plurals",
    source: PLURAL_EN,
    locale: "pl",
    target: "{count, plural, one {# nieprzeczytana wiadomość} other {# nieprzeczytanych wiadomości}}",
  },
  { id: "tag", tag: "<b>", source: "Read the <b>docs</b> first", locale: "de", target: "Lies zuerst die Doku" },
  { id: "syntax", tag: t("syntax.icu_label"), source: t("file_count.plural"), locale: "ja", target: "{count, plural, other {# 個のファイル}" },
  { id: "arabic", tag: "plurals", source: PLURAL_EN, locale: "ar", target: "{count, plural, one {رسالة واحدة غير مقروءة} other {# رسائل غير مقروءة}}" },
] as const;
}
const LOCALES = ["fr", "de", "es", "pl", "ru", "ja", "zh", "ar"];

export function Playground() {
  const CHALLENGES = useChallenges();
  const t = useTranslations("landing.play");
  const root = useRef<HTMLDivElement>(null);
  const [validate, setValidate] = useState<Validate | null>(null);
  const [failed, setFailed] = useState(false);
  const [source, setSource] = useState<string>(CHALLENGES[0].source);
  const [locale, setLocale] = useState<string>(CHALLENGES[0].locale);
  const [target, setTarget] = useState<string>(CHALLENGES[0].target);
  const [refused, setRefused] = useState(0);
  const seen = useRef(new Set<string>());
  const [typed, setTyped] = useState(false);

  const load = useCallback(() => {
    if (validate || failed) return;
    loadValidator()
      .then((v) => setValidate(() => v))
      .catch(() => setFailed(true));
  }, [validate, failed]);

  // Fetch the 270 kB validator only when the section gets close to the viewport.
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const io = new IntersectionObserver((e) => e[0].isIntersecting && (load(), io.disconnect()), { rootMargin: "400px" });
    io.observe(el);
    return () => io.disconnect();
  }, [load]);

  const deferred = useDeferredValue(target);
  const input = useMemo(() => ({ source, target: deferred, locale }), [source, deferred, locale]);
  const issues = useMemo(() => (validate ? validate(input) : null), [validate, input]);
  const errors = issues?.filter((i) => i.severity === "error") ?? [];
  const warnings = issues?.filter((i) => i.severity === "warning") ?? [];

  // Gamification on real data: each distinct translation the validator refused counts once.
  useEffect(() => {
    if (!typed || !errors.length) return;
    const k = `${input.locale}\u0000${input.source}\u0000${input.target}`;
    if (seen.current.has(k)) return;
    const id = setTimeout(() => {
      seen.current.add(k);
      setRefused(seen.current.size);
    }, 700);
    return () => clearTimeout(id);
  }, [typed, errors.length, input]);

  const pick = (c: (ReturnType<typeof useChallenges>)[number]) => {
    setSource(c.source);
    setLocale(c.locale);
    setTarget(c.target);
    setTyped(false);
  };

  const field = "w-full rounded-lg border bg-background px-3 py-2.5 font-mono text-sm leading-relaxed shadow-xs transition-[border-color,box-shadow] outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/30";

  return (
    <div ref={root} className="reveal grid gap-4 rounded-2xl border bg-card p-4 shadow-sm sm:p-6 lg:grid-cols-[1fr_1fr]">
      <div className="space-y-4">
        <div>
          <p id="pg-challenges" className="mb-2 text-xs font-medium text-muted-foreground">
            {t("challenges")}
          </p>
          <div role="group" aria-labelledby="pg-challenges" className="flex flex-wrap gap-2">
            {CHALLENGES.map((c) => {
              const active = c.target === target && c.source === source && c.locale === locale;
              return (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => pick(c)}
                  aria-pressed={active}
                  dir="ltr"
                  className={cn(
                    "min-h-9 rounded-full border px-3 font-mono text-xs transition-colors duration-150 hover:bg-muted",
                    active && "border-foreground/40 bg-muted",
                  )}
                >
                  <span className="me-1.5 text-muted-foreground">{c.locale}</span>
                  {c.tag}
                </button>
              );
            })}
          </div>
        </div>
        <div>
          <label htmlFor="pg-source" className="mb-1.5 block text-sm font-medium">
            {t("source")}
          </label>
          <textarea id="pg-source" dir="ltr" lang="en" rows={2} spellCheck={false} value={source} onChange={(e) => { setSource(e.target.value); setTyped(true); }} onFocus={load} className={cn(field, "resize-y")} />
        </div>
        <div className="grid grid-cols-[1fr_auto] items-end gap-3">
          <label htmlFor="pg-target" className="block text-sm font-medium">
            {t("target")}
          </label>
          <div className="flex items-center gap-2">
            <label htmlFor="pg-locale" className="text-xs text-muted-foreground">
              {t("locale")}
            </label>
            <select id="pg-locale" value={locale} onChange={(e) => setLocale(e.target.value)} className="h-9 rounded-md border bg-background px-2 font-mono text-sm shadow-xs">
              {LOCALES.map((l) => (
                <option key={l} value={l}>
                  {l}
                </option>
              ))}
            </select>
          </div>
          <textarea
            id="pg-target"
            dir={locale === "ar" ? "rtl" : "ltr"}
            lang={locale}
            rows={3}
            spellCheck={false}
            value={target}
            onChange={(e) => { setTarget(e.target.value); setTyped(true); }}
            onFocus={load}
            aria-invalid={errors.length > 0}
            aria-describedby="pg-result"
            className={cn(field, "col-span-2 resize-y", errors.length > 0 && "border-destructive/60")}
          />
        </div>
        <button
          type="button"
          onClick={() => pick(CHALLENGES[0])}
          className="inline-flex min-h-9 items-center gap-1.5 rounded-md px-2 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <RotateCcw aria-hidden className="size-3.5" /> {t("reset")}
        </button>
      </div>

      <div className="flex min-h-72 flex-col rounded-xl border term p-4 font-mono text-[12.5px] leading-relaxed">
        <div id="pg-result" role="status" aria-live="polite" className="flex-1">
          {failed ? (
            <p className="term-err">{t("failed")}</p>
          ) : !issues ? (
            <p className="term-dim">{t("loading")}</p>
          ) : (
            <>
              <p key={errors.length ? "no" : "ok"} className={cn("animate-rise flex items-center gap-2 font-sans text-sm font-semibold", errors.length ? "term-err" : "term-ok")}>
                {errors.length ? <CircleX aria-hidden className="size-4" /> : <CircleCheck aria-hidden className="size-4" />}
                {errors.length ? t("refused", { count: errors.length }) : t("accepted")}
              </p>
              <ul dir="ltr" lang="en" className="mt-3 space-y-2.5">
                {[...errors, ...warnings].map((i, n) => (
                  <li key={`${i.code}-${n}`} className={cn("flex gap-2", i.severity === "error" ? "term-err" : "term-warn")}>
                    {i.severity === "error" ? <CircleX aria-hidden className="mt-1 size-3.5 shrink-0" /> : <TriangleAlert aria-hidden className="mt-1 size-3.5 shrink-0" />}
                    <span>
                      <span className="term-faint">{i.code}</span> {i.message}
                    </span>
                  </li>
                ))}
              </ul>
              {warnings.length > 0 && <p className="mt-3 font-sans text-xs term-dim">{t("warnings", { count: warnings.length })}</p>}
            </>
          )}
        </div>
        <p className="term-rule mt-4 border-t pt-3 font-sans text-xs term-dim">{t("score", { refused })}</p>
      </div>
      <p className="text-xs text-muted-foreground lg:col-span-2">{t("note")}</p>
    </div>
  );
}
