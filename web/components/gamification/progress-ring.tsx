"use client";

import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/**
 * Circular progress (0–100). Fills from 0 on mount, then animates between values
 * (stroke-dashoffset only; reduced motion = instant via the global media query).
 * Pass `label` for screen readers; children render centered (e.g. a locale code).
 */
export function ProgressRing({
  value,
  label,
  size = 36,
  stroke = 3,
  className,
  children,
}: {
  value: number;
  label: string;
  size?: number;
  stroke?: number;
  className?: string;
  children?: React.ReactNode;
}) {
  const [shown, setShown] = useState(0);
  useEffect(() => {
    const id = requestAnimationFrame(() => setShown(Math.max(0, Math.min(100, value))));
    return () => cancelAnimationFrame(id);
  }, [value]);
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const done = value >= 100;
  return (
    <span role="img" aria-label={label} className={cn("relative inline-grid shrink-0 place-items-center", className)} style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden className="absolute inset-0 -rotate-90 rtl:-scale-x-100">
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" strokeWidth={stroke} className="stroke-muted" />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - shown / 100)}
          className={cn(
            "transition-[stroke-dashoffset,stroke] duration-(--duration-data) ease-(--ease-out-quint)",
            done ? "stroke-success" : "stroke-brand",
            shown === 0 && "opacity-0",
          )}
        />
      </svg>
      {children && <span aria-hidden className="relative text-[10px] leading-none font-medium">{children}</span>}
    </span>
  );
}

/** Counts up to `value` (requestAnimationFrame, ~600 ms ease-out). Screen readers get the final value only. */
export function AnimatedNumber({ value, format = String }: { value: number; format?: (n: number) => string }) {
  const [n, setN] = useState(0);
  const cur = useRef(0);
  useEffect(() => {
    const from = cur.current;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const start = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      const t = reduce ? 1 : Math.min(1, (now - start) / 600);
      cur.current = from + (value - from) * (1 - Math.pow(1 - t, 5));
      setN(cur.current);
      if (t < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value]);
  return (
    <>
      <span aria-hidden className="tabular-nums">{format(Math.round(n))}</span>
      <span className="sr-only">{format(value)}</span>
    </>
  );
}
