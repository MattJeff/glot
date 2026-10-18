// Landing page building blocks (server components). Product output shown in the visuals (glot's refusal
// messages, benchmark numbers, command lines) is data, not UI copy: it lives in constants. Every label
// goes through next-intl.
import {
  ArrowRight,
  BookOpenText,
  Check,
  CircleAlert,
  CircleX,
  FileLock2,
  type LucideIcon,
  Ruler,
  ScanSearch,
  ShieldCheck,
  SquareFunction,
  Braces,
  Wand2,
  Languages,
  Workflow,
} from "lucide-react";
import Image from "next/image";
import { getFormatter, getTranslations } from "next-intl/server";
import { LocaleSwitcher, Logo, ThemeToggle } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import { GITHUB_URL } from "@/lib/snippets";
import { cn } from "@/lib/utils";

export const LOCALIZE = "npx glot-i18n localize";
export const APP_URL = "https://app.glotlang.com";
export const NPM_URL = "https://www.npmjs.com/package/glot-i18n";

// bench/results/2026-10-18-real/report.md: 320 messages per model and condition, 8 locales.
export const BENCH = {
  models: [
    { model: "Claude Haiku 5.5", raw: 0.141, glot: 0, rejected: 0.019 },
    { model: "Claude Sonnet 5.5", raw: 0.088, glot: 0, rejected: 0.031 },
  ],
  max: 0.15,
};

/** Live-ish project facts for the hero (revalidated hourly). Any failure just hides the number. */
export async function getRepoStats(): Promise<{ stars: number | null; version: string | null }> {
  const get = async (url: string) => {
    try {
      const r = await fetch(url, { next: { revalidate: 3600 }, signal: AbortSignal.timeout(3000) });
      return r.ok ? await r.json() : null;
    } catch {
      return null;
    }
  };
  const [gh, npm] = await Promise.all([
    get(GITHUB_URL.replace("https://github.com/", "https://api.github.com/repos/")),
    get("https://registry.npmjs.org/glot-i18n/latest"),
  ]);
  return {
    stars: typeof gh?.stargazers_count === "number" ? gh.stargazers_count : null,
    version: typeof npm?.version === "string" ? npm.version : null,
  };
}

export function SectionHead({ eyebrow, title, subtitle, id, center }: { eyebrow?: string; title: string; subtitle?: string; id?: string; center?: boolean }) {
  return (
    <div className={cn("reveal max-w-2xl", center && "mx-auto text-center")}>
      {eyebrow && <p className="text-sm font-medium text-brand">{eyebrow}</p>}
      <h2 id={id} className="mt-2 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
        {title}
      </h2>
      {subtitle && <p className="mt-3 text-base text-pretty text-muted-foreground sm:text-lg">{subtitle}</p>}
    </div>
  );
}

/** Benchmark proof: horizontal bars, direct labels, a table for assistive tech. */
export async function BenchChart() {
  const t = await getTranslations("landing.bench");
  const f = await getFormatter();
  const pct = (v: number) => f.number(v, { style: "percent", minimumFractionDigits: v ? 1 : 0, maximumFractionDigits: 1 });
  return (
    <figure aria-labelledby="bench-title" className="flex h-full flex-col rounded-xl border bg-card p-5 shadow-xs sm:p-6">
      <p className="text-xs font-medium text-muted-foreground">{t("eyebrow")}</p>
      <h2 id="bench-title" className="mt-1 text-lg font-semibold tracking-tight">
        {t("title")}
      </h2>
      <p className="mt-1 text-sm text-muted-foreground">{t("lead")}</p>

      <div aria-hidden className="mt-5 flex items-baseline gap-3">
        <span className="bench-strike relative text-2xl font-semibold text-muted-foreground tabular-nums">{pct(BENCH.models[0].raw)}</span>
        <ArrowRight className="size-4 self-center text-muted-foreground rtl:rotate-180" />
        <span className="bench-zero text-5xl font-semibold tracking-tight text-brand tabular-nums">{pct(0)}</span>
      </div>

      <ul aria-hidden className="mt-4 flex gap-4 text-xs text-muted-foreground">
        <li className="flex items-center gap-1.5">
          <span className="size-2.5 rounded-sm bg-foreground/35" /> {t("raw")}
        </li>
        <li className="flex items-center gap-1.5">
          <span className="size-2.5 rounded-sm bg-brand" /> {t("glot")}
        </li>
      </ul>

      <div aria-hidden className="mt-3 space-y-4">
        {BENCH.models.map((m, i) => (
          <div key={m.model}>
            <p className="mb-1.5 text-xs font-medium">{m.model}</p>
            {(["raw", "glot"] as const).map((c) => (
              <div key={c} className="flex h-6 items-center gap-2">
                <div className="relative h-3.5 flex-1">
                  {m[c] > 0 ? (
                    <div
                      className="bench-bar h-full rounded-e-[4px] bg-foreground/35"
                      style={{ width: `${(m[c] / BENCH.max) * 100}%`, animationDelay: `${300 + i * 150}ms` }}
                    />
                  ) : (
                    <div className="h-full w-0.5 rounded-full bg-brand" />
                  )}
                </div>
                <span className={cn("w-12 text-end text-xs tabular-nums", c === "glot" ? "font-semibold text-brand" : "text-muted-foreground")}>
                  {pct(m[c])}
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>

      <div className="sr-only">
      <table>
        <caption>{t("tableCaption")}</caption>
        <thead>
          <tr>
            <th scope="col">{t("model")}</th>
            <th scope="col">{t("raw")}</th>
            <th scope="col">{t("glot")}</th>
          </tr>
        </thead>
        <tbody>
          {BENCH.models.map((m) => (
            <tr key={m.model}>
              <th scope="row">{m.model}</th>
              <td>{pct(m.raw)}</td>
              <td>{pct(m.glot)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      </div>

      <figcaption className="mt-auto pt-5 text-xs leading-relaxed text-muted-foreground">
        {t("rejected", { haiku: pct(BENCH.models[0].rejected), sonnet: pct(BENCH.models[1].rejected) })}{" "}
        <Link href="/docs/benchmark" className="font-medium text-foreground underline underline-offset-2 hover:text-brand">
          {t("report")}
        </Link>
      </figcaption>
    </figure>
  );
}

const STEPS: { key: "init" | "i18nize" | "translate"; icon: LucideIcon; cmd: string; out: string[] }[] = [
  { key: "init", icon: ScanSearch, cmd: "npx glot-i18n init", out: ["next-intl · messages/{locale}.json", "wrote glot.toml, CLAUDE.md rules"] },
  { key: "i18nize", icon: Wand2, cmd: "npx glot-i18n i18nize", out: ["42 strings → keys in 9 components", "app/checkout/page.tsx  +12 −12"] },
  { key: "translate", icon: Languages, cmd: "npx glot-i18n translate", out: ["✗ refused pl inbox.unread: missing few, many", "✓ fixed  pl attempt 2 · check passed"] },
];

export async function HowSteps() {
  const t = await getTranslations("landing.how");
  return (
    <ol className="mt-12 grid gap-4 md:grid-cols-3">
      {STEPS.map(({ key, icon: Icon, cmd, out }, i) => (
        <li key={key} className="reveal step relative flex min-w-0 flex-col rounded-xl border bg-card p-5 shadow-xs" style={{ animationDelay: `${i * 80}ms` }}>
          <div className="flex items-center gap-2.5">
            <span className="grid size-7 place-items-center rounded-full border bg-background font-mono text-xs tabular-nums">{i + 1}</span>
            <Icon aria-hidden className="size-4 text-brand" />
            <h3 className="font-medium">{t(`${key}.title`)}</h3>
          </div>
          <p className="mt-3 flex-1 text-sm leading-relaxed text-muted-foreground">{t(`${key}.body`)}</p>
          <div dir="ltr" className="mt-5 overflow-hidden rounded-lg border term font-mono text-xs">
            <p className="truncate border-b term-rule px-3 py-2">
              <span className="term-faint select-none">$ </span>
              {cmd}
            </p>
            <div className="space-y-1 px-3 py-2.5">
              {out.map((l, j) => (
                <p key={l} className={cn("step-line truncate", l.startsWith("✗") ? "term-err" : l.startsWith("✓") ? "term-ok" : "term-dim")} style={{ animationDelay: `${i * 80 + 250 + j * 200}ms` }}>
                  {l}
                </p>
              ))}
            </div>
          </div>
          {i < STEPS.length - 1 && (
            <ArrowRight aria-hidden className="absolute top-9 -end-3.5 z-10 hidden size-5 rounded-full border bg-background p-0.5 text-muted-foreground md:block rtl:rotate-180" />
          )}
        </li>
      ))}
    </ol>
  );
}

// Real validator output (glot-core, same text as the CLI prints).
const GUARANTEES: { key: "placeholders" | "plurals" | "glossary" | "width" | "lockfile" | "ci"; icon: LucideIcon; bad: string; msg: string }[] = [
  {
    key: "placeholders",
    icon: Braces,
    bad: `fr  "Bon retour, {nom} !"`,
    msg: "Placeholder `{nom}` does not exist in the source; remove it or fix its name (the source only has {name}).",
  },
  {
    key: "plurals",
    icon: SquareFunction,
    bad: `pl  "{count, plural, one {…} other {…}}"`,
    msg: "`{count, plural, …}` is missing plural categories required by pl: few, many.",
  },
  {
    key: "glossary",
    icon: BookOpenText,
    bad: `fr  "Ouvrez votre espace"`,
    msg: "Glossary: `Workspace` must be translated as `Espace de travail` in fr.",
  },
  {
    key: "width",
    icon: Ruler,
    bad: `de  "Zahlungserinnerung an den Kunden senden"`,
    msg: "Too wide for its box in the app: one line of 120px at Geist 14px/400; it renders 276px wide.",
  },
  {
    key: "lockfile",
    icon: FileLock2,
    bad: `$ glot translate --locales fr`,
    msg: "1 edited by hand",
  },
  {
    key: "ci",
    icon: Workflow,
    bad: `$ glot check --format github`,
    msg: "::error file=messages/de.json,line=12,title=glot tag_mismatch::[de] home.cta: Markup tags differ from the source: missing `<b>`",
  },
];

export async function GuaranteeGrid() {
  const t = await getTranslations("landing.guarantees");
  return (
    <ul className="mt-12 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {GUARANTEES.map(({ key, icon, bad, msg }, i) => (
        <RefusalCard key={key} id={`g-${key}`} i={i} icon={icon} title={t(`${key}.title`)} body={t(`${key}.body`)} bad={bad} msg={msg} refusal={t("refusal")} />
      ))}
    </ul>
  );
}

/** One guarantee: what it checks, then a real refusal (product output, shown verbatim). */
export function RefusalCard({ id, i, icon: Icon, title, body, bad, msg, refusal }: { id: string; i: number; icon: LucideIcon; title: string; body: string; bad: string; msg: string; refusal: string }) {
  return (
    <li className="reveal" style={{ animationDelay: `${(i % 3) * 60}ms` }}>
      {/* Focusable so keyboard users get the same reveal as hover. */}
      <article
        tabIndex={0}
        aria-labelledby={id}
        className="guarantee group flex h-full flex-col rounded-xl border bg-card p-5 shadow-xs transition-[border-color,box-shadow,translate] duration-200 ease-out outline-none hover:-translate-y-0.5 hover:border-destructive/40 hover:shadow-md focus-visible:border-destructive/40 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none motion-reduce:hover:translate-y-0"
      >
        <div className="flex items-center gap-2.5">
          <span className="grid size-8 place-items-center rounded-lg border bg-background">
            <Icon aria-hidden className="size-4 text-brand" />
          </span>
          <h3 id={id} className="font-medium">
            {title}
          </h3>
        </div>
        <p className="mt-3 flex-1 text-sm leading-relaxed text-muted-foreground">{body}</p>
        <div dir="ltr" lang="en" className="mt-4 overflow-hidden rounded-lg border term p-3 font-mono text-[11.5px] leading-relaxed">
          <p className="truncate term-dim transition-colors duration-200 group-hover:opacity-60 group-hover:line-through group-focus-visible:opacity-60 group-focus-visible:line-through">
            {bad}
          </p>
          <p className="guarantee-msg mt-1.5 flex gap-1.5 term-err">
            <CircleX aria-hidden className="mt-0.5 size-3.5 shrink-0" />
            <span>
              <span className="sr-only">{refusal}: </span>
              {msg}
            </span>
          </p>
        </div>
      </article>
    </li>
  );
}

// German UI text from the demo app run (product data).
const FIT = { before: "Zahlungserinnerung senden", after: "Mahnung", primary: "Als bezahlt markieren", archive: "Archivieren", cmd: "$ glot fit --locales de", run: "claude-haiku-5-5 · $0.0002" };

/** Real `glot fit` run on the demo app (docs/launch/fit-before-after.png), rendered live in HTML. */
export async function FitDemo() {
  const t = await getTranslations("landing.fit");
  const row = (label: string, fits: boolean) => (
    <div dir="ltr" lang="de" className="flex flex-wrap items-center justify-center gap-2 rounded-lg border bg-muted/50 p-4 text-sm">
      <span
        className={cn(
          "fit-btn grid h-10 w-28 place-items-center rounded-md border bg-background px-2 font-medium shadow-xs",
          !fits && "outline-2 outline-offset-2 outline-dashed outline-destructive/70",
        )}
      >
        <span className="block max-w-full truncate">{label}</span>
      </span>
      <span className="grid h-10 place-items-center rounded-md bg-brand px-3 font-medium text-brand-foreground">{FIT.primary}</span>
      <span className="grid h-10 place-items-center rounded-md border bg-background px-3 text-muted-foreground">{FIT.archive}</span>
    </div>
  );
  return (
    <div className="reveal rounded-xl border bg-card p-4 shadow-xs sm:p-6">
      <figure>
        <figcaption className="mb-2 flex items-center gap-1.5 text-xs font-medium">
          <CircleAlert aria-hidden className="size-3.5 text-destructive" />
          {t("before")} · <span className="font-normal text-muted-foreground">{t("overflow")}</span>
        </figcaption>
        {row(FIT.before, false)}
        <p dir="ltr" className="mt-1.5 font-mono text-[11px] text-muted-foreground">{`"${FIT.before}"`}</p>
      </figure>
      <div aria-hidden className="my-3 flex justify-center">
        <span dir="ltr" className="rounded-full border term px-3 py-1 font-mono text-[11px]">
          {FIT.cmd}
        </span>
      </div>
      <figure className="fit-after">
        <figcaption className="mb-2 flex items-center gap-1.5 text-xs font-medium">
          <Check aria-hidden className="size-3.5 text-success" />
          {t("after")} · <span className="font-normal text-muted-foreground">{t("fits")}</span>
        </figcaption>
        {row(FIT.after, true)}
        <p dir="ltr" className="mt-1.5 font-mono text-[11px] text-muted-foreground">{`"${FIT.after}" · ${FIT.run}`}</p>
      </figure>
      <p className="mt-4 text-xs text-muted-foreground">{t("meta")}</p>
    </div>
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

/** Text column + visual column; `flip` swaps sides on large screens. */
export function Showcase({
  eyebrow,
  title,
  body,
  points,
  flip,
  id,
  children,
}: {
  eyebrow: string;
  title: string;
  body: string;
  points: string[];
  flip?: boolean;
  id?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid items-center gap-10 lg:grid-cols-2 lg:gap-16">
      <div className={cn("reveal", flip && "lg:order-2")}>
        <p className="text-sm font-medium text-brand">{eyebrow}</p>
        <h2 id={id} className="mt-2 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
          {title}
        </h2>
        <p className="mt-3 text-pretty text-muted-foreground">{body}</p>
        <Points items={points} />
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

export async function OverlayVisual() {
  const t = await getTranslations("landing.overlay");
  return (
    <div className="reveal overflow-hidden rounded-xl border bg-muted/40 p-2 shadow-sm">
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

// Agent names are product names (text wordmarks, no third-party logos). Setup paths: docs/integrations, docs/skills.
const AGENTS: { brand: string; how: "plugin" | "rules" | "agentsMd" | "skill" | "prompt"; href: string }[] = [
  { brand: "Claude Code", how: "plugin", href: "/docs/integrations#claude-code" },
  { brand: "Cursor", how: "rules", href: "/docs/integrations#cursor" },
  { brand: "Codex", how: "agentsMd", href: "/docs/integrations#codex" },
  { brand: "Windsurf", how: "rules", href: "/docs/integrations#cursor" },
  { brand: "Gemini CLI", how: "skill", href: "/docs/skills" },
  { brand: "GitHub Copilot", how: "skill", href: "/docs/skills" },
  { brand: "Lovable", how: "prompt", href: "/docs/integrations#lovable-bolt-v0-replit" },
  { brand: "Bolt", how: "prompt", href: "/docs/integrations#lovable-bolt-v0-replit" },
  { brand: "v0", how: "prompt", href: "/docs/integrations#lovable-bolt-v0-replit" },
];

export async function AgentsGrid() {
  const t = await getTranslations("landing.agents");
  return (
    <>
      <ul className="mt-12 grid grid-cols-2 gap-px overflow-hidden rounded-xl border bg-border sm:grid-cols-3">
        {AGENTS.map((a) => (
          <li key={a.brand} className="bg-background">
            <Link
              href={a.href}
              className="group flex h-full min-h-20 flex-col justify-center gap-1 px-5 py-4 transition-colors duration-150 hover:bg-muted/60 focus-visible:bg-muted/60 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
            >
              <span className="text-base font-semibold tracking-tight">{a.brand}</span>
              <span className="flex items-center gap-1 text-xs text-muted-foreground">
                {t(a.how)}
                <ArrowRight aria-hidden className="size-3 opacity-0 transition-opacity group-hover:opacity-100 rtl:rotate-180" />
              </span>
            </Link>
          </li>
        ))}
      </ul>
      <p className="mt-4 flex items-center gap-1.5 text-sm text-muted-foreground">
        <ShieldCheck aria-hidden className="size-4 text-brand" />
        <Link href="/docs/integrations" className="underline underline-offset-2 hover:text-foreground">
          {t("docs")}
        </Link>
      </p>
    </>
  );
}

export async function Faq() {
  const t = await getTranslations("landing.faq");
  const items = ["cost", "runtime", "fails", "code", "app"] as const;
  return (
    <div className="divide-y rounded-xl border bg-card">
      {items.map((k) => (
        <details key={k} className="faq group">
          <summary className="flex min-h-14 cursor-pointer list-none items-center justify-between gap-4 px-5 py-4 font-medium hover:bg-muted/40 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring [&::-webkit-details-marker]:hidden">
            {t(`${k}.q`)}
            <span aria-hidden className="grid size-6 shrink-0 place-items-center rounded-full border text-muted-foreground transition-transform duration-200 group-open:rotate-45">
              +
            </span>
          </summary>
          <p className="px-5 pb-5 text-sm leading-relaxed text-muted-foreground">{t(`${k}.a`)}</p>
        </details>
      ))}
    </div>
  );
}

/** GitHub mark (for links to the repo, per GitHub's logo guidelines). */
export function GithubMark({ className }: { className?: string }) {
  return (
    <svg aria-hidden viewBox="0 0 16 16" fill="currentColor" className={className}>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

type NavLink = { href: string; label: string; route?: boolean };

/** Chrome labels, resolved by each page from its own namespace (so /i18n keeps its translations). */
export type Chrome = { skip: string; home: string; nav: string; docs: string; pricing: string; openApp: string; stars: string; tagline: string; npm: string; license: string };

/** Sticky marketing header (home and /i18n). `route` links go through next-intl, others are in-page anchors. */
export function SiteHeader({ c, links, stars }: { c: Chrome; links: NavLink[]; stars: number | null }) {
  const item = "rounded-md px-2.5 py-1.5 transition-colors hover:bg-muted hover:text-foreground";
  return (
    <>
      <a href="#main" className="sr-only z-50 rounded-md bg-background px-3 py-2 focus:not-sr-only focus:fixed focus:start-3 focus:top-3 focus:shadow-lg">
        {c.skip}
      </a>
      <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/70">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-3 px-4 sm:gap-6">
          <Link href="/" aria-label={c.home}>
            <Logo />
          </Link>
          <nav aria-label={c.nav} className="hidden items-center gap-1 text-sm text-muted-foreground lg:flex">
            {links.map((l) =>
              l.route ? (
                <Link key={l.href} href={l.href} className={item}>
                  {l.label}
                </Link>
              ) : (
                <a key={l.href} href={l.href} className={item}>
                  {l.label}
                </a>
              ),
            )}
            <Link href="/docs" className={item}>
              {c.docs}
            </Link>
            <Link href="/pricing" className={item}>
              {c.pricing}
            </Link>
          </nav>
          <div className="ms-auto flex items-center gap-1">
            <Link href="/docs" className="rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground lg:hidden">
              {c.docs}
            </Link>
            <a
              href={GITHUB_URL}
              target="_blank"
              rel="noreferrer"
              aria-label={c.stars}
              className="hidden h-8 items-center gap-1.5 rounded-md px-2 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground sm:inline-flex"
            >
              <GithubMark className="size-4" />
              {stars ? <span className="tabular-nums">{stars}</span> : null}
            </a>
            <LocaleSwitcher />
            <ThemeToggle />
            <Button size="sm" asChild className="ms-1">
              <a href={APP_URL}>{c.openApp}</a>
            </Button>
          </div>
        </div>
      </header>
    </>
  );
}

export async function SiteFooter({ c, links }: { c: Chrome; links: NavLink[] }) {
  const tc = await getTranslations("common");
  return (
    <footer className="border-t">
      <div className="mx-auto flex max-w-6xl flex-wrap items-center gap-x-6 gap-y-3 px-4 py-8 text-sm text-muted-foreground">
        <Logo className="text-foreground" />
        <span>{c.tagline}</span>
        <nav aria-label={c.docs} className="ms-auto flex flex-wrap items-center gap-x-5 gap-y-2">
          {links.map((l) => (
            <Link key={l.href} href={l.href} className="inline-flex min-h-6 items-center hover:text-foreground">
              {l.label}
            </Link>
          ))}
          <a href={NPM_URL} target="_blank" rel="noreferrer" className="inline-flex min-h-6 items-center hover:text-foreground">{c.npm}</a>
          <a href={GITHUB_URL} target="_blank" rel="noreferrer" className="inline-flex min-h-6 items-center hover:text-foreground">{tc("github")}</a>
          <span>{c.license}</span>
        </nav>
      </div>
    </footer>
  );
}
