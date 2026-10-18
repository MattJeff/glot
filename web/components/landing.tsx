// Landing page building blocks (server components). Sample data shown in the mock visuals is
// product data, not UI copy, so it lives in constants; every label goes through next-intl.
import { ArrowRight, ChartColumn, Check, CircleAlert, Cpu, Languages, ShieldCheck, Undo2 } from "lucide-react";
import Image from "next/image";
import { getFormatter, getLocale, getTranslations } from "next-intl/server";
import { cn } from "@/lib/utils";

// ponytail: illustrative numbers, labelled "Example data" on screen.
const DEMAND = [
  { locale: "de", share: 0.21, served: true },
  { locale: "fr", share: 0.17, served: true },
  { locale: "pt-BR", share: 0.12, served: false },
  { locale: "ja", share: 0.08, served: true },
  { locale: "it", share: 0.06, served: false },
];

const REVIEW_SAMPLE = { source: "Send reminder", target: "Envoyer une relance", back: "Send a follow-up", locale: "fr" };

const MATRIX = [
  { locale: "en", sample: "Send reminder", issues: 0 },
  { locale: "de", sample: "Erinnerung senden", issues: 1 },
  { locale: "fr", sample: "Envoyer une relance", issues: 0 },
  { locale: "ja", sample: "リマインダーを送信", issues: 0 },
  { locale: "ar", sample: "إرسال تذكير", issues: 0, rtl: true },
  { locale: "es", sample: "Enviar recordatorio", issues: 1 },
];

export const COMMANDS = {
  i18nize: "glot i18nize --yes",
  translate: "glot translate",
  check: "glot check --format github",
  fit: "glot fit --locales de",
};

/** Real recording (docs/launch/record/refusal.sh): the LLM is scripted to fail, the refusals are glot's. */
export function RefusalDemo({ label, caption }: { label: string; caption: string }) {
  return (
    <figure className="min-w-0">
      <div dir="ltr" className="overflow-hidden rounded-xl border bg-[#0b0f17] shadow-sm">
        <video
          src="/launch/refusal.webm"
          poster="/launch/refusal.png"
          autoPlay
          muted
          loop
          playsInline
          aria-label={label}
          width={1100}
          height={680}
          className="h-auto w-full"
        />
      </div>
      <figcaption className="mt-2 text-xs text-muted-foreground">{caption}</figcaption>
    </figure>
  );
}

export function Points({ items }: { items: string[] }) {
  return (
    <ul className="mt-6 space-y-2.5 text-sm">
      {items.map((p) => (
        <li key={p} className="flex gap-2.5">
          <Check aria-hidden className="mt-0.5 size-4 shrink-0 text-brand" />
          <span>{p}</span>
        </li>
      ))}
    </ul>
  );
}

/** Text column + visual column, visual first on the end side; `flip` swaps sides on large screens. */
export function Showcase({
  eyebrow,
  title,
  body,
  points,
  flip,
  children,
}: {
  eyebrow: string;
  title: string;
  body: string;
  points: string[];
  flip?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="grid items-center gap-10 lg:grid-cols-2 lg:gap-16">
      <div className={cn(flip && "lg:order-2")}>
        <p className="text-sm font-medium text-brand">{eyebrow}</p>
        <h3 className="mt-2 text-2xl font-semibold tracking-tight text-balance sm:text-3xl">{title}</h3>
        <p className="mt-3 text-pretty text-muted-foreground">{body}</p>
        <Points items={points} />
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

export async function FitVisual() {
  const t = await getTranslations("landing.fit");
  const shot = (src: string, alt: string, label: string, w: number, h: number, bad: boolean) => (
    <figure className="flex flex-1 flex-col items-center gap-3 rounded-lg border bg-background p-4">
      <figcaption className="flex items-center gap-1.5 self-start text-xs font-medium">
        {bad ? <CircleAlert aria-hidden className="size-3.5 text-warning" /> : <Check aria-hidden className="size-3.5 text-success" />}
        {label}
      </figcaption>
      <div className="grid h-36 w-full place-items-center overflow-hidden rounded-md bg-white p-2">
        <Image src={src} alt={alt} width={w * 2} height={h * 2} unoptimized className="h-auto max-h-full w-auto max-w-full object-contain" />
      </div>
    </figure>
  );
  return (
    <div className="rounded-xl border bg-muted/40 p-4 sm:p-6">
      <div className="flex flex-col items-stretch gap-3 sm:flex-row sm:items-center">
        {shot("/landing/fit-before.png", t("beforeAlt"), t("before"), 128, 72, true)}
        <ArrowRight aria-hidden className="mx-auto size-4 shrink-0 rotate-90 text-muted-foreground sm:rotate-0 rtl:sm:rotate-180" />
        {shot("/landing/fit-after.png", t("afterAlt"), t("after"), 160, 104, false)}
      </div>
      <pre dir="ltr" className="mt-4 overflow-x-auto rounded-md border bg-background px-3 py-2 font-mono text-xs">
        <span className="text-muted-foreground select-none">$ </span>
        {COMMANDS.fit}
      </pre>
      <p className="mt-3 text-xs text-muted-foreground">{t("caption")}</p>
    </div>
  );
}

export async function OverlayVisual() {
  const t = await getTranslations("landing.overlay");
  return (
    <div className="overflow-hidden rounded-xl border bg-muted/40 p-2 shadow-sm">
      <Image
        src="/landing/popover-light.png"
        alt={t("alt")}
        width={1400}
        height={897}
        sizes="(min-width: 1024px) 560px, 100vw"
        className="rounded-lg border dark:hidden"
      />
      <Image
        src="/landing/popover-dark-ai.png"
        alt={t("alt")}
        width={1400}
        height={897}
        sizes="(min-width: 1024px) 560px, 100vw"
        className="hidden rounded-lg border dark:block"
      />
    </div>
  );
}

function Tile({ icon: Icon, title, body, children }: { icon: React.ComponentType<{ className?: string }>; title: string; body: string; children: React.ReactNode }) {
  return (
    <article className="flex flex-col rounded-xl border bg-card p-5 sm:p-6">
      <div className="flex items-center gap-2">
        <Icon aria-hidden className="size-4 text-brand" />
        <h3 className="font-medium">{title}</h3>
      </div>
      <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{body}</p>
      <div className="mt-5 flex-1">{children}</div>
    </article>
  );
}

export async function ReviewTile() {
  const t = await getTranslations("landing.review");
  const s = REVIEW_SAMPLE;
  return (
    <Tile icon={Undo2} title={t("title")} body={t("body")}>
      <dl className="space-y-2 rounded-lg border bg-background p-3 text-sm">
        <div className="flex gap-3">
          <dt className="w-28 shrink-0 text-xs text-muted-foreground">{t("source")}</dt>
          <dd>{s.source}</dd>
        </div>
        <div className="flex gap-3">
          <dt className="w-28 shrink-0 font-mono text-xs text-muted-foreground uppercase">{s.locale}</dt>
          <dd lang={s.locale}>{s.target}</dd>
        </div>
        <div className="flex gap-3">
          <dt className="w-28 shrink-0 text-xs text-muted-foreground">{t("back")}</dt>
          <dd className="flex flex-wrap items-center gap-2">
            {s.back}
            <span className="rounded-full bg-warning/15 px-2 py-0.5 text-xs font-medium text-warning">{t("nuance")}</span>
          </dd>
        </div>
        <p className="border-t pt-2 text-xs leading-relaxed text-muted-foreground">{t("explanation")}</p>
      </dl>
    </Tile>
  );
}

export async function MatrixTile() {
  const t = await getTranslations("landing.matrix");
  return (
    <Tile icon={Languages} title={t("title")} body={t("body")}>
      <ul className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {MATRIX.map((m) => (
          <li key={m.locale} className={cn("rounded-lg border bg-background p-2", m.issues > 0 && "border-warning/60")}>
            <div className="mb-2 flex items-center justify-between gap-1">
              <span className="font-mono text-[11px] text-muted-foreground uppercase">{m.locale}</span>
              {m.issues > 0 ? (
                <CircleAlert aria-label={t("issues", { count: m.issues })} className="size-3.5 text-warning" />
              ) : (
                <Check aria-label={t("ok")} className="size-3.5 text-success" />
              )}
            </div>
            <div
              lang={m.locale}
              dir={m.rtl ? "rtl" : "ltr"}
              className={cn(
                "mx-auto truncate rounded-md border px-2 py-1.5 text-center text-[11px] text-foreground",
                // Overflowing tiles render in a narrower box so the truncation is visible.
                m.issues > 0 ? "w-3/4 border-dashed border-warning" : "w-full",
              )}
            >
              {m.sample}
            </div>
          </li>
        ))}
      </ul>
    </Tile>
  );
}

export async function InsightsTile() {
  const t = await getTranslations("landing.insights");
  const f = await getFormatter();
  const ui = await getLocale();
  const name = (code: string) => {
    try {
      return new Intl.DisplayNames([ui], { type: "language" }).of(code) ?? code;
    } catch {
      return code;
    }
  };
  return (
    <Tile icon={ChartColumn} title={t("title")} body={t("body")}>
      <div className="rounded-lg border bg-background p-3">
        <p className="mb-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{t("sample")}</p>
        <ul className="space-y-2.5">
          {DEMAND.map((d) => (
            <li key={d.locale} className="text-xs">
              <div className="mb-1 flex items-center justify-between gap-2">
                <span className="truncate">{name(d.locale)}</span>
                <span className={cn("shrink-0", d.served ? "text-muted-foreground" : "font-medium text-warning")}>
                  {d.served ? t("served") : t("notServed")}
                </span>
              </div>
              <div
                role="img"
                aria-label={t("share", { value: f.number(d.share, { style: "percent" }) })}
                className="h-1.5 overflow-hidden rounded-full bg-muted"
              >
                <div className={cn("h-full rounded-full", d.served ? "bg-brand" : "bg-warning")} style={{ width: `${d.share * 400}%` }} />
              </div>
            </li>
          ))}
        </ul>
      </div>
    </Tile>
  );
}

export async function OnDeviceTile() {
  const t = await getTranslations("landing.ondevice");
  const steps = [
    { icon: Languages, text: t("step1") },
    { icon: ShieldCheck, text: t("step2") },
    { icon: Undo2, text: t("step3") },
  ];
  return (
    <Tile icon={Cpu} title={t("title")} body={t("body")}>
      <ol className="space-y-2">
        {steps.map(({ icon: Icon, text }, i) => (
          <li key={text} className="flex items-center gap-3 rounded-lg border bg-background p-3 text-sm">
            <span className="grid size-6 shrink-0 place-items-center rounded-md border bg-muted font-mono text-[11px]">{i + 1}</span>
            <Icon aria-hidden className="size-4 shrink-0 text-brand" />
            <span>{text}</span>
          </li>
        ))}
      </ol>
    </Tile>
  );
}

export function FlowStep({
  n,
  icon: Icon,
  label,
  caption,
  children,
}: {
  n: number;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  caption: string;
  children: React.ReactNode;
}) {
  return (
    <li className="relative flex min-w-0 flex-col rounded-xl border bg-card p-4 shadow-xs">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid size-6 place-items-center rounded-md border bg-muted font-mono text-[11px]">{n}</span>
        <Icon className="size-4 text-brand" />
        <span className="text-sm font-medium">{label}</span>
      </div>
      <div className="flex-1">{children}</div>
      <p className="mt-3 text-xs text-muted-foreground">{caption}</p>
      {n < 4 && (
        <ArrowRight aria-hidden className="absolute top-1/2 -end-3 z-10 hidden size-4 -translate-y-1/2 rounded-full bg-background text-muted-foreground md:block rtl:rotate-180" />
      )}
    </li>
  );
}
