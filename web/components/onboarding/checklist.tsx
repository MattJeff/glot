"use client";

import {
  BookA,
  Bot,
  Check,
  Circle,
  CircleCheck,
  Globe,
  KeyRound,
  Loader2,
  Lock,
  Medal,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  GitBranch,
  Users,
  type LucideIcon,
} from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { CodeBlock, ErrorState, useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Link, useRouter } from "@/i18n/navigation";
import { useProject } from "@/lib/hooks";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import {
  ACHIEVEMENTS,
  type AchievementId,
  type Onboarding,
  type OnboardingPath,
  type StepId,
  useCreateOnboardingToken,
  useOnboarding,
  usePatchOnboarding,
} from "./api";
import * as cmd from "./commands";
import { confetti } from "./confetti";
import { GithubSlot } from "./github-slot";
import { NextActionCard } from "./next-action";
import { ProgressRing } from "./progress-ring";

// Commands wrap instead of scrolling: no hidden overflow, nothing to scroll by keyboard, fine at 320 px.
const WRAP = "[&_pre]:whitespace-pre-wrap [&_pre]:[overflow-wrap:anywhere]";

const PATHS: { id: OnboardingPath; icon: LucideIcon }[] = [
  { id: "agent", icon: Bot },
  { id: "terminal", icon: SquareTerminal },
  { id: "github", icon: GitBranch },
];

export function OnboardingChecklist({ slug }: { slug: string }) {
  const t = useTranslations("onboarding");
  const project = useProject(slug);
  const ob = useOnboarding(slug, true);
  const patch = usePatchOnboarding(slug);
  const router = useRouter();
  const [token, setToken] = useState<string | null>(null);
  useCelebrations(ob.data);

  if (ob.isError) return <ErrorState error={ob.error} onRetry={() => ob.refetch()} />;
  if (!ob.data || !project.data) return <ChecklistSkeleton />;

  const o = ob.data;
  const done = (id: StepId) => !!o.steps.find((s) => s.id === id)?.done;
  const doneCount = o.steps.filter((s) => s.done).length;
  const pct = Math.round((doneCount / o.steps.length) * 100);
  const path = o.path ?? "agent";
  const ctx: cmd.Ctx = { server: o.server_url, slug: o.slug, token: token ?? "glot_…", targets: o.locales.map((l) => l.locale) };

  return (
    <div className="mx-auto max-w-5xl">
      <header className="mb-8 flex flex-wrap items-center gap-4">
        <ProgressRing value={pct} size={56} stroke={5} label={t("progress", { done: doneCount, total: o.steps.length })}>
          {doneCount}/{o.steps.length}
        </ProgressRing>
        <div className="min-w-0 flex-1 basis-64">
          <h1 className="text-xl font-semibold tracking-tight text-balance">{t("title", { name: project.data.name })}</h1>
          <p className="mt-1 text-sm text-pretty text-muted-foreground">{t("subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              patch.mutate({ hidden: true });
              router.push(`/app/${slug}`);
            }}
          >
            {t("hide")}
          </Button>
          <Button variant="outline" size="sm" asChild>
            <Link href={`/app/${slug}`}>{t("toDashboard")}</Link>
          </Button>
        </div>
      </header>

      <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_17rem]">
        <ol className="space-y-4" aria-label={t("stepsLabel")}>
          <StepCard n={1} done={done("path")} title={t("path.title")} description={t("path.description")}>
            <PathPicker value={path} onChange={(p) => patch.mutate({ path: p })} />
          </StepCard>

          <StepCard n={2} done={done("token")} title={t(`connect.${path}.title`)} description={t(`connect.${path}.description`)}>
            {path === "github" ? (
              <GithubSlot slug={slug} />
            ) : (
              <div className="space-y-4">
                <TokenPanel slug={slug} token={token} onToken={setToken} hadToken={done("token")} />
                {path === "agent" ? (
                  <>
                    <CodeBlock className={WRAP} label={t("connect.agent.claude")} code={cmd.claudeCode(ctx)} />
                    <CodeBlock className={WRAP} label={t("connect.agent.json")} code={cmd.mcpJson(ctx)} />
                    <CodeBlock className={WRAP} label={t("connect.agent.prompt")} code={cmd.agentPrompt(ctx)} />
                  </>
                ) : (
                  <>
                    <CodeBlock className={WRAP} label={t("connect.terminal.localize")} code={cmd.localize(ctx)} />
                    <CodeBlock className={WRAP} label={t("connect.terminal.push")} code={cmd.push(ctx)} />
                  </>
                )}
              </div>
            )}
          </StepCard>

          <StepCard n={3} done={done("first_push")} title={t("wait.title")} description={t("wait.description")}>
            <LiveWait o={o} />
          </StepCard>

          <StepCard n={4} done={o.steps.slice(3).every((s) => s.done)} title={t("more.title")} description={t("more.description")}>
            {o.keys > 0 && (
              <div className="mb-4">
                <NextActionCard slug={slug} onboarding={o} />
              </div>
            )}
            <FollowUps o={o} ctx={ctx} />
          </StepCard>
        </ol>

        <aside className="space-y-6" aria-label={t("sideLabel")}>
          <LocaleRings o={o} />
          <Achievements o={o} />
        </aside>
      </div>
    </div>
  );
}

function StepCard({
  n,
  done,
  title,
  description,
  children,
}: {
  n: number;
  done: boolean;
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  const t = useTranslations("onboarding");
  return (
    <li className="rounded-xl border bg-card p-4 sm:p-5 animate-in fade-in-0 slide-in-from-bottom-1 duration-300 motion-reduce:animate-none">
      <div className="mb-4 flex items-start gap-3">
        <span
          aria-hidden
          className={cn(
            "grid size-7 shrink-0 place-items-center rounded-full border font-mono text-xs transition-colors duration-200",
            done ? "border-success bg-success text-background" : "text-muted-foreground",
          )}
        >
          {done ? <Check className="size-4 animate-in zoom-in-50 duration-200 motion-reduce:animate-none" strokeWidth={3} /> : n}
        </span>
        <div className="min-w-0">
          <h2 className="font-medium">
            {title}
            <span className="sr-only"> ({done ? t("stepDone") : t("stepTodo")})</span>
          </h2>
          <p className="mt-0.5 text-sm text-pretty text-muted-foreground">{description}</p>
        </div>
      </div>
      <div className="sm:ps-10">{children}</div>
    </li>
  );
}

function PathPicker({ value, onChange }: { value: OnboardingPath; onChange: (p: OnboardingPath) => void }) {
  const t = useTranslations("onboarding.path");
  return (
    <fieldset>
      <legend className="sr-only">{t("title")}</legend>
      <div className="grid gap-2 sm:grid-cols-3">
        {PATHS.map(({ id, icon: Icon }) => (
          <label key={id} className="relative block cursor-pointer">
            <input
              type="radio"
              name="onboarding-path"
              value={id}
              checked={value === id}
              onChange={() => onChange(id)}
              className="peer sr-only"
            />
            <span className="flex h-full flex-col gap-1.5 rounded-lg border p-3 transition-colors duration-150 hover:bg-accent/50 peer-checked:border-brand peer-checked:bg-brand/5 peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-background">
              <span className="flex items-center gap-2 text-sm font-medium whitespace-nowrap">
                <Icon aria-hidden className="size-4 shrink-0 text-brand" />
                {t(`${id}.name`)}
                {id === "agent" && (
                  <Badge variant="secondary" className="ms-auto h-5 px-1.5 text-[11px]">
                    {t("recommended")}
                  </Badge>
                )}
              </span>
              <span className="text-xs text-muted-foreground">{t(`${id}.hint`)}</span>
            </span>
            <CircleCheck
              aria-hidden
              className="absolute end-2 bottom-2 size-4 text-brand opacity-0 transition-opacity duration-150 peer-checked:opacity-100"
            />
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function TokenPanel({
  slug,
  token,
  onToken,
  hadToken,
}: {
  slug: string;
  token: string | null;
  onToken: (t: string) => void;
  hadToken: boolean;
}) {
  const t = useTranslations("onboarding.token");
  const create = useCreateOnboardingToken(slug);
  const message = useErrorMessage();
  const ref = useRef<HTMLDivElement>(null);

  if (token) {
    return (
      <div ref={ref} tabIndex={-1} className="space-y-2 rounded-lg border border-brand/30 bg-brand/5 p-3 outline-none">
        <p className="flex items-center gap-2 text-sm font-medium">
          <KeyRound aria-hidden className="size-4 text-brand" />
          {t("created")}
        </p>
        <p className="text-xs text-muted-foreground">{t("once")}</p>
        <CodeBlock code={token} className={cn(WRAP, "bg-background")} />
      </div>
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-lg border border-dashed p-3">
      <KeyRound aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <p className="min-w-0 flex-1 basis-56 text-sm text-muted-foreground">{hadToken ? t("again") : t("why")}</p>
      <Button
        size="sm"
        variant={hadToken ? "outline" : "default"}
        disabled={create.isPending}
        aria-busy={create.isPending}
        onClick={() =>
          create.mutate(undefined, {
            onSuccess: (r) => {
              onToken(r.token);
              toast.success(t("created"));
              // The panel re-renders with the token: move focus to it so keyboard and screen reader users land there.
              requestAnimationFrame(() => ref.current?.focus());
            },
            onError: (e) => toast.error(message(e)),
          })
        }
      >
        {create.isPending ? <Loader2 aria-hidden className="animate-spin" /> : <KeyRound aria-hidden />}
        {hadToken ? t("regenerate") : t("generate")}
      </Button>
    </div>
  );
}

function LiveWait({ o }: { o: Onboarding }) {
  const t = useTranslations("onboarding.wait");
  const format = useFormatter();
  const now = useNow({ updateInterval: 10_000 });
  const [slow, setSlow] = useState(false);
  const heading = useRef<HTMLParagraphElement>(null);
  const waited = useRef(o.keys === 0);
  const received = o.keys > 0;

  useEffect(() => {
    if (received) return;
    const id = setTimeout(() => setSlow(true), 30_000);
    return () => clearTimeout(id);
  }, [received]);
  useEffect(() => {
    // Focus follows the flow: when the first push lands live, move to the result.
    if (received && waited.current) heading.current?.focus();
  }, [received]);

  return (
    <div className="space-y-3">
      <div
        role="status"
        aria-live="polite"
        className={cn(
          "flex items-center gap-3 rounded-lg border p-4 transition-colors duration-300",
          received && "border-success/40 bg-success/5",
        )}
      >
        {received ? (
          <CircleCheck aria-hidden className="size-5 shrink-0 text-success animate-in zoom-in-50 duration-300 motion-reduce:animate-none" />
        ) : (
          <span className="relative flex size-2.5 shrink-0" aria-hidden>
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-brand opacity-60 motion-reduce:hidden" />
            <span className="relative inline-flex size-2.5 rounded-full bg-brand" />
          </span>
        )}
        <p ref={heading} tabIndex={-1} className="text-sm outline-none">
          {received ? (
            <>
              <span className="font-medium">
                {t("received", { keys: o.keys, languages: o.locales.length })}
              </span>
              {o.first_push_at && (
                <span className="text-muted-foreground"> · {format.relativeTime(new Date(o.first_push_at), now)}</span>
              )}
            </>
          ) : (
            t("waiting")
          )}
        </p>
      </div>
      {!received && slow && (
        <div className="rounded-lg bg-muted/50 p-3 text-sm text-muted-foreground animate-in fade-in-0 duration-300">
          <p className="font-medium text-foreground">{t("slowTitle")}</p>
          <ul className="mt-1 list-disc space-y-0.5 ps-5">
            <li>{t("slowToken")}</li>
            <li>{t("slowProject", { slug: o.slug })}</li>
            <li>{t("slowServer", { url: o.server_url })}</li>
          </ul>
        </div>
      )}
    </div>
  );
}

const FOLLOW_UPS = [
  { id: "first_translation", icon: Sparkles },
  { id: "zero_errors", icon: ShieldCheck },
  { id: "invite", icon: Users },
  { id: "glossary", icon: BookA },
  { id: "cdn", icon: Globe },
  { id: "badge", icon: Medal },
] as const;

function FollowUps({ o, ctx }: { o: Onboarding; ctx: cmd.Ctx }) {
  const t = useTranslations("onboarding.more");
  const base = `/app/${o.slug}`;
  const done = (id: StepId) => !!o.steps.find((s) => s.id === id)?.done;
  const cta: Partial<Record<StepId, [string, string]>> = {
    first_translation: [`${base}/jobs`, t("first_translation.cta")],
    zero_errors: [base, t("zero_errors.cta")],
    glossary: [`${base}/glossary`, t("glossary.cta")],
    cdn: [`${base}/settings`, t("cdn.cta")],
  };
  return (
    <ul className="divide-y rounded-lg border">
      {FOLLOW_UPS.map(({ id, icon: Icon }: { id: (typeof FOLLOW_UPS)[number]["id"]; icon: LucideIcon }) => {
        const isDone = done(id);
        return (
          <li key={id} className="flex flex-wrap items-start gap-3 p-3">
            {isDone ? (
              <CircleCheck aria-hidden className="mt-0.5 size-4 shrink-0 text-success" />
            ) : (
              <Circle aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            )}
            <div className="min-w-0 flex-1 basis-56">
              <p className={cn("flex items-center gap-1.5 text-sm font-medium", isDone && "text-muted-foreground")}>
                <Icon aria-hidden className="size-3.5" />
                {t(`${id}.title`)}
                <span className="sr-only"> ({isDone ? t("done") : t("todo")})</span>
              </p>
              <p className="mt-0.5 text-xs text-muted-foreground">{t(`${id}.body`)}</p>
              {id === "badge" && done("cdn") && <CodeBlock code={cmd.badge(ctx)} className={cn(WRAP, "mt-2")} />}
            </div>
            {!isDone && cta[id] && (
              <Button variant="ghost" size="sm" asChild>
                <Link href={cta[id][0]}>
                  {cta[id][1]}
                  <span className="sr-only"> ({t(`${id}.title`)})</span>
                </Link>
              </Button>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function LocaleRings({ o }: { o: Onboarding }) {
  const t = useTranslations("onboarding.locales");
  const name = useLocaleName();
  const format = useFormatter();
  return (
    <section aria-labelledby="ob-locales">
      <h2 id="ob-locales" className="mb-3 text-sm font-medium">
        {t("title")}
      </h2>
      {o.locales.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("none")}</p>
      ) : (
        <ul className="space-y-2">
          {o.locales.map((l) => {
            const pct = o.keys ? Math.round((Math.min(l.translated, o.keys) / o.keys) * 100) : 0;
            return (
              <li key={l.locale}>
                <Link
                  href={`/app/${o.slug}/keys?locale=${l.locale}`}
                  className="flex items-center gap-3 rounded-md p-1 hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                >
                  <ProgressRing value={pct} size={36} label={t("ring", { language: name(l.locale), value: pct })} />
                  <span className="min-w-0 flex-1 truncate text-sm capitalize">{name(l.locale)}</span>
                  <span className="font-mono text-xs tabular-nums text-muted-foreground">
                    {format.number(pct / 100, { style: "percent" })}
                  </span>
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

export function Achievements({ o, compact }: { o: Onboarding; compact?: boolean }) {
  const t = useTranslations("onboarding.achievements");
  const format = useFormatter();
  const unlocked = new Map(o.achievements.map((a) => [a.id, a.unlocked_at]));
  const list = compact ? ACHIEVEMENTS.filter((id) => unlocked.has(id)) : ACHIEVEMENTS;
  if (compact && list.length === 0) return null;
  return (
    <section aria-labelledby={compact ? undefined : "ob-achievements"} aria-label={compact ? t("title") : undefined}>
      {!compact && (
        <h2 id="ob-achievements" className="mb-3 text-sm font-medium">
          {t("title")} <span className="font-normal text-muted-foreground tabular-nums">{unlocked.size}/{ACHIEVEMENTS.length}</span>
        </h2>
      )}
      <ul className={compact ? "flex flex-wrap gap-2" : "space-y-2"}>
        {list.map((id) => {
          const at = unlocked.get(id);
          return (
            <li
              key={id}
              className={cn(
                "flex items-center gap-2.5 rounded-lg border text-sm transition-colors duration-300",
                compact ? "px-2.5 py-1" : "p-2.5",
                at ? "border-success/40 bg-success/5" : "border-dashed text-muted-foreground",
              )}
            >
              {at ? (
                <Medal aria-hidden className="size-4 shrink-0 text-success" />
              ) : (
                <Lock aria-hidden className="size-4 shrink-0" />
              )}
              <span className="min-w-0">
                <span className={cn("block", at && "font-medium")}>{t(`${id}.name`)}</span>
                {!compact && (
                  <span className="block text-xs text-muted-foreground">
                    {at ? t("unlocked", { date: format.dateTime(new Date(at), { dateStyle: "medium" }) }) : t(`${id}.how`)}
                  </span>
                )}
              </span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** Toast each milestone reached while the page is open; confetti once, when the first push lands live. */
function useCelebrations(o: Onboarding | undefined) {
  const t = useTranslations("onboarding.achievements");
  const seen = useRef<Set<AchievementId> | null>(null);
  useEffect(() => {
    if (!o) return;
    if (!seen.current) {
      seen.current = new Set(o.achievements.map((a) => a.id));
      return;
    }
    for (const a of o.achievements) {
      if (seen.current.has(a.id)) continue;
      seen.current.add(a.id);
      toast.success(t("toast", { name: t(`${a.id}.name`) }), { description: t(`${a.id}.how`) });
      if (a.id === "first_push") confetti();
    }
  }, [o, t]);
}

export function ChecklistSkeleton() {
  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <div className="flex items-center gap-4">
        <Skeleton className="size-14 rounded-full" />
        <Skeleton className="h-10 w-72" />
      </div>
      <div className="grid gap-8 lg:grid-cols-[minmax(0,1fr)_17rem]">
        <div className="space-y-4">
          {Array.from({ length: 3 }, (_, i) => (
            <Skeleton key={i} className="h-40 rounded-xl" />
          ))}
        </div>
        <Skeleton className="h-64 rounded-xl" />
      </div>
    </div>
  );
}

