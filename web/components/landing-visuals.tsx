"use client";

import { Check } from "lucide-react";
import { useEffect, useState } from "react";
import { ACCENTS } from "@/components/landing-accents";

// Decorative visuals for the landing. Content is the product's own sample copy in each language,
// so it is not in the UI catalogs. Both freeze on their first frame with prefers-reduced-motion.

function useTick(ms: number, n: number) {
  const [i, setI] = useState(0);
  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const id = setInterval(() => setI((x) => (x + 1) % n), ms);
    return () => clearInterval(id);
  }, [ms, n]);
  return i;
}

const MORPH = [
  { code: "en", text: "Get paid faster. Send invoices in minutes." },
  { code: "fr", text: "Soyez payé plus vite. Vos factures en quelques minutes." },
  { code: "de", text: "Schneller bezahlt. Rechnungen in Minuten." },
  { code: "ja", text: "もっと早く入金を。請求書は数分で。" },
  { code: "es", text: "Cobra antes. Envía facturas en minutos." },
  { code: "ar", text: "احصل على أموالك أسرع. فواتير في دقائق." },
  { code: "pt", text: "Receba mais rápido. Faturas em minutos." },
  { code: "hi", text: "जल्दी भुगतान पाएं। मिनटों में इनवॉइस भेजें।" },
];
const CHECKS = ["{placeholders}", "plural rules", "glossary", "fits 160px", "no fake urgency"];

/** One sentence travelling through languages while the validator ticks its checks. */
export function LangMorph() {
  const i = useTick(2600, MORPH.length);
  const m = MORPH[i];
  return (
    <div aria-hidden className={`morph card-acc relative overflow-hidden rounded-2xl border bg-card p-6 shadow-xs sm:p-8 ${ACCENTS[i % ACCENTS.length]}`}>
      <div className="flex flex-wrap gap-1.5">
        {MORPH.map((x, j) => (
          <span key={x.code} className={`rounded-full px-2 py-0.5 font-mono text-[11px] transition-colors duration-300 ${j === i ? "tile" : "text-muted-foreground"}`}>
            {x.code}
          </span>
        ))}
      </div>
      <p key={m.code} lang={m.code} dir="auto" className="morph-text mt-6 min-h-[4.5rem] text-2xl font-semibold tracking-tight text-balance sm:text-3xl">
        {m.text}
      </p>
      <ul key={`c${m.code}`} className="mt-6 flex flex-wrap gap-2" dir="ltr">
        {CHECKS.map((c, j) => (
          <li key={c} className="morph-check inline-flex items-center gap-1 rounded-full border px-2.5 py-1 font-mono text-[11px] text-muted-foreground" style={{ animationDelay: `${300 + j * 180}ms` }}>
            <Check className="acc-ink size-3" /> {c}
          </li>
        ))}
      </ul>
    </div>
  );
}

const MARKETS = [
  { m: "de-DE", w: 0, lift: 18 },
  { m: "fr-FR", w: 1, lift: 11 },
  { m: "ja-JP", w: 2, lift: 24 },
  { m: "pt-BR", w: 1, lift: 9 },
  { m: "es-MX", w: 0, lift: 14 },
  { m: "en-US", w: 2, lift: 6 },
  { m: "ar-SA", w: 1, lift: 21 },
  { m: "hi-IN", w: 0, lift: 16 },
];
const VARIANT_ACC = ["acc-cyan", "acc-coral", "acc-violet"];

/** Each market crowns its own variant; the confidence bars fill one market at a time. */
export function WinnerBoard() {
  const lit = useTick(900, MARKETS.length);
  return (
    <ul aria-hidden className="grid grid-cols-2 gap-3 sm:grid-cols-4" dir="ltr">
      {MARKETS.map(({ m, w, lift }, j) => (
        <li key={m} className={`card-acc rounded-xl border bg-card p-4 shadow-xs ${VARIANT_ACC[w]} ${j === lit ? "board-lit" : ""}`}>
          <div className="flex items-center justify-between">
            <span className="font-mono text-xs text-muted-foreground">{m}</span>
            <span className="tile size-7 rounded-full font-mono text-xs font-semibold">{"ABC"[w]}</span>
          </div>
          <p className="acc-ink mt-3 text-2xl font-semibold tabular-nums">+{lift}%</p>
          <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
            <div className="board-bar h-full rounded-full" style={{ width: `${70 + lift}%`, background: "var(--acc)" }} />
          </div>
        </li>
      ))}
    </ul>
  );
}
