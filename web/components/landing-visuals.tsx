"use client";

import { useEffect, useState } from "react";

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
