import { cn } from "@/lib/utils";

/** SVG progress ring; `label` is the accessible name (the visible % is optional children). */
export function ProgressRing({
  value,
  label,
  size = 40,
  stroke = 4,
  children,
  className,
}: {
  value: number;
  label: string;
  size?: number;
  stroke?: number;
  children?: React.ReactNode;
  className?: string;
}) {
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const v = Math.max(0, Math.min(100, value));
  return (
    <span className={cn("relative inline-grid shrink-0 place-items-center", className)} style={{ width: size, height: size }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={label} className="-rotate-90">
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" strokeWidth={stroke} className="stroke-muted" />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - v / 100)}
          className={cn(
            "transition-[stroke-dashoffset] duration-700 ease-out motion-reduce:transition-none",
            v >= 100 ? "stroke-success" : "stroke-brand",
            v === 0 && "opacity-0",
          )}
        />
      </svg>
      {children && (
        <span aria-hidden className="absolute inset-0 grid place-items-center font-mono text-[11px] font-medium tabular-nums">
          {children}
        </span>
      )}
    </span>
  );
}
