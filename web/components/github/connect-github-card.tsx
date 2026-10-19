"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  CircleCheck,
  ExternalLink,
  GitBranch,
  GitPullRequest,
  Loader2,
  Lock,
  RotateCw,
  Search,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useId, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { Link } from "@/i18n/navigation";
import { api } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";
import { ghKey, githubApi, STEPS, type GithubJob, type GithubStatus } from "./api";

/** GitHub mark (brand logo of the integration; lucide has no brand icons). */
export function GitHubMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden className={cn("fill-current", className)}>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8" />
    </svg>
  );
}

const active = (j: GithubJob | null | undefined) => !!j && (j.status === "queued" || j.status === "running");

/** Live job progress over the existing jobs SSE stream, patched into the status query. */
function useLiveJob(slug: string, job: GithubJob | null | undefined) {
  const qc = useQueryClient();
  const id = job?.id;
  const live = active(job);
  useEffect(() => {
    if (!id || !live) return;
    const es = new EventSource(api.jobEventsUrl(id));
    const patch = (e: Event) => {
      try {
        const next = JSON.parse((e as MessageEvent<string>).data) as GithubJob;
        qc.setQueryData<GithubStatus>(ghKey.status(slug), (s) => (s ? { ...s, job: next } : s));
      } catch {
        /* ignore malformed frame */
      }
    };
    es.addEventListener("progress", patch);
    es.addEventListener("done", (e) => {
      patch(e);
      es.close();
      qc.invalidateQueries({ queryKey: ghKey.status(slug) });
    });
    // Polling (refetchInterval) takes over if the stream drops.
    es.onerror = () => es.close();
    return () => es.close();
  }, [id, live, slug, qc]);
}

/** « Connect GitHub »: install the app → pick a repo → live job → pull request. Self-contained (onboarding checklist, github page). */
export function ConnectGitHubCard({ slug, className }: { slug: string; className?: string }) {
  const t = useTranslations("github");
  const titleId = useId();
  const status = useQuery({
    queryKey: ghKey.status(slug),
    queryFn: () => githubApi.status(slug),
    refetchInterval: (q) => (active(q.state.data?.job) ? 3000 : false),
  });
  const [picking, setPicking] = useState(false);
  useLiveJob(slug, status.data?.job);
  const s = status.data;

  let body: React.ReactNode;
  if (status.isPending) body = <Skeleton className="h-24 rounded-lg" />;
  else if (status.isError) body = <Failure error={status.error} onRetry={() => status.refetch()} />;
  else if (!s!.app) body = <NoApp />;
  else if (s!.installations.length === 0) body = <Install slug={slug} />;
  else if (!s!.link || picking) body = <RepoPicker slug={slug} status={s!} onDone={() => setPicking(false)} />;
  else body = <Linked slug={slug} status={s!} onChangeRepo={() => setPicking(true)} />;

  return (
    <section aria-labelledby={titleId} className={cn("rounded-xl border bg-card p-5 shadow-(--shadow-card)", className)}>
      <div className="mb-4 flex items-start gap-3">
        <span className="grid size-10 shrink-0 place-items-center rounded-lg border bg-background">
          <GitHubMark className="size-5" />
        </span>
        <div className="min-w-0">
          <h2 id={titleId} className="text-base font-medium">
            {t("card.title")}
          </h2>
          <p className="text-sm text-pretty text-muted-foreground">{t("card.subtitle")}</p>
        </div>
      </div>
      {body}
    </section>
  );
}

function Failure({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const tc = useTranslations("common");
  const message = useErrorMessage();
  return (
    <div role="alert" className="flex items-center gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm">
      <TriangleAlert className="size-4 shrink-0 text-destructive" aria-hidden />
      <p className="min-w-0 flex-1">{message(error)}</p>
      <Button variant="outline" size="sm" onClick={onRetry}>
        <RotateCw /> {tc("retry")}
      </Button>
    </div>
  );
}

function NoApp() {
  const t = useTranslations("github");
  const app = useQuery({ queryKey: ghKey.app, queryFn: githubApi.app });
  return (
    <div className="rounded-lg border border-dashed p-4 text-sm">
      <p className="font-medium">{t("noApp.title")}</p>
      <p className="mt-1 text-muted-foreground">{t("noApp.body")}</p>
      {app.data?.can_admin ? (
        <Button asChild size="sm" className="mt-3">
          <Link href="/admin/github-app">
            <GitHubMark className="size-4" /> {t("noApp.setup")}
          </Link>
        </Button>
      ) : (
        <Link href="/docs/github" className="mt-2 inline-block font-medium text-foreground underline underline-offset-4">
          {t("noApp.docs")}
        </Link>
      )}
    </div>
  );
}

function Promises() {
  const t = useTranslations("github");
  return (
    <ul className="space-y-1.5 text-sm text-muted-foreground">
      {(["pr", "noCode", "updates"] as const).map((k) => (
        <li key={k} className="flex items-start gap-2">
          <Check className="mt-0.5 size-4 shrink-0 text-success-foreground" aria-hidden />
          {t(`promise.${k}`)}
        </li>
      ))}
    </ul>
  );
}

function Install({ slug }: { slug: string }) {
  const t = useTranslations("github");
  const message = useErrorMessage();
  const connect = useMutation({
    mutationFn: () => githubApi.connect(slug),
  });
  const go = (which: "install_url" | "authorize_url") =>
    connect.mutate(undefined, { onSuccess: (urls) => window.location.assign(urls[which]) });
  return (
    <div className="space-y-4">
      <Promises />
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <Button onClick={() => go("install_url")} disabled={connect.isPending} aria-busy={connect.isPending}>
          {connect.isPending ? <Loader2 className="animate-spin" /> : <GitHubMark className="size-4" />}
          {connect.isPending ? t("install.redirecting") : t("install.cta")}
        </Button>
        <button
          type="button"
          onClick={() => go("authorize_url")}
          disabled={connect.isPending}
          className="rounded-sm text-sm text-muted-foreground underline underline-offset-4 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          {t("install.already")}
        </button>
      </div>
      {connect.isError && (
        <p role="alert" className="text-sm text-destructive">
          {message(connect.error)}
        </p>
      )}
    </div>
  );
}

function RepoPicker({ slug, status, onDone }: { slug: string; status: GithubStatus; onDone: () => void }) {
  const t = useTranslations("github");
  const message = useErrorMessage();
  const qc = useQueryClient();
  const ids = { search: useId(), branch: useId(), branchHelp: useId(), account: useId() };
  const [installation, setInstallation] = useState(status.link?.installation_id ?? status.installations[0].id);
  const [q, setQ] = useState("");
  const [repo, setRepo] = useState<string | null>(status.link?.repo ?? null);
  const [branch, setBranch] = useState(status.link?.branch ?? "");
  const repos = useQuery({
    queryKey: ghKey.repos(slug, installation),
    queryFn: () => githubApi.repos(slug, installation),
  });
  const link = useMutation({
    mutationFn: () => githubApi.link(slug, { installation_id: installation, repo: repo!, branch: branch || undefined }),
    onSuccess: ({ link, job }) => {
      qc.setQueryData<GithubStatus>(ghKey.status(slug), (s) => (s ? { ...s, link, job } : s));
      onDone();
    },
  });
  const shown = (repos.data ?? []).filter((r) => r.full_name.toLowerCase().includes(q.trim().toLowerCase()));
  const chosen = repos.data?.find((r) => r.full_name === repo);

  return (
    <form
      className="space-y-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (repo) link.mutate();
      }}
    >
      {status.installations.length > 1 && (
        <div className="space-y-1.5">
          <Label htmlFor={ids.account}>{t("pick.account")}</Label>
          <select
            id={ids.account}
            value={installation}
            onChange={(e) => {
              setInstallation(Number(e.target.value));
              setRepo(null);
            }}
            className="h-9 w-full rounded-md border bg-background px-3 text-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            {status.installations.map((i) => (
              <option key={i.id} value={i.id}>
                {i.account}
              </option>
            ))}
          </select>
        </div>
      )}
      <fieldset className="space-y-1.5">
        <legend className="mb-1.5 text-sm font-medium">{t("pick.repo")}</legend>
        <div className="relative">
          <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
          <Input
            id={ids.search}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder={t("pick.search")}
            aria-label={t("pick.search")}
            className="ps-8"
          />
        </div>
        {repos.isPending ? (
          <div className="space-y-1.5" aria-busy>
            {Array.from({ length: 3 }, (_, i) => (
              <Skeleton key={i} className="h-10 rounded-md" />
            ))}
          </div>
        ) : repos.isError ? (
          <Failure error={repos.error} onRetry={() => repos.refetch()} />
        ) : shown.length === 0 ? (
          <p className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
            {repos.data.length === 0 ? t("pick.none") : t("pick.noMatch")}
          </p>
        ) : (
          <div className="max-h-64 space-y-1 overflow-y-auto rounded-md border p-1">
            {shown.map((r, i) => (
              <label
                key={r.full_name}
                style={{ animationDelay: `${Math.min(i, 8) * 30}ms` }}
                className={cn(
                  "flex min-h-10 cursor-pointer items-center gap-3 rounded-md px-3 text-sm transition-colors duration-150 hover:bg-accent has-focus-visible:ring-2 has-focus-visible:ring-ring motion-safe:animate-rise",
                  repo === r.full_name && "bg-accent",
                )}
              >
                <input
                  type="radio"
                  name="repo"
                  value={r.full_name}
                  checked={repo === r.full_name}
                  onChange={() => {
                    setRepo(r.full_name);
                    setBranch(r.default_branch);
                  }}
                  className="size-4 accent-(--brand)"
                />
                <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-[13px]">
                  {r.full_name}
                </span>
                {r.private && (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Lock className="size-3" aria-hidden /> {t("pick.private")}
                  </span>
                )}
              </label>
            ))}
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          {t("pick.missing")}{" "}
          <a
            href={`${status.app!.html_url}/installations/new`}
            className="font-medium text-foreground underline underline-offset-4"
            target="_blank"
            rel="noreferrer"
          >
            {t("pick.addRepos")}
          </a>
        </p>
      </fieldset>
      {repo && (
        <div className="space-y-1.5 motion-safe:animate-rise">
          <Label htmlFor={ids.branch}>{t("pick.branch")}</Label>
          <div className="relative">
            <GitBranch className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
            <Input
              id={ids.branch}
              dir="ltr"
              value={branch}
              onChange={(e) => setBranch(e.target.value)}
              placeholder={chosen?.default_branch}
              aria-describedby={ids.branchHelp}
              className="ps-8 font-mono"
            />
          </div>
          <p id={ids.branchHelp} className="text-xs text-muted-foreground">
            {t("pick.branchHelp")}
          </p>
        </div>
      )}
      {link.isError && (
        <p role="alert" className="text-sm text-destructive">
          {message(link.error)}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={!repo || link.isPending} aria-busy={link.isPending}>
          {link.isPending ? <Loader2 className="animate-spin" /> : <GitPullRequest />}
          {repo ? t("pick.cta", { repo: repo.split("/")[1] }) : t("pick.ctaEmpty")}
        </Button>
        {status.link && (
          <Button type="button" variant="ghost" onClick={onDone}>
            {t("pick.cancel")}
          </Button>
        )}
      </div>
    </form>
  );
}

function Steps({ job }: { job: GithubJob }) {
  const t = useTranslations("github");
  const done = job.progress.done;
  const current = Math.min(done, STEPS.length - 1);
  const pct = (done / STEPS.length) * 100;
  return (
    <div className="space-y-3">
      <Progress value={pct} aria-label={t("job.progress", { done, total: STEPS.length })} className="h-1.5" />
      <ol className="space-y-1.5">
        {STEPS.map((step, i) => {
          const state = i < done ? "done" : i === current ? "current" : "todo";
          return (
            <li
              key={step}
              aria-current={state === "current" ? "step" : undefined}
              className={cn(
                "flex items-center gap-2.5 text-sm transition-colors duration-200",
                state === "todo" && "text-muted-foreground",
              )}
            >
              <span className="grid size-5 shrink-0 place-items-center" aria-hidden>
                {state === "done" ? (
                  <CircleCheck className="size-4 text-success-foreground motion-safe:animate-pop" />
                ) : state === "current" ? (
                  <Loader2 className="size-4 animate-spin text-info-foreground" />
                ) : (
                  <span className="size-2 rounded-full bg-border" />
                )}
              </span>
              <span className={cn(state === "current" && "font-medium")}>{t(`step.${step}`)}</span>
              <span className="sr-only">{t(`stepState.${state}`)}</span>
            </li>
          );
        })}
      </ol>
      <p role="status" aria-live="polite" className="sr-only">
        {t("job.announce", { step: t(`step.${STEPS[current]}`), n: current + 1, total: STEPS.length })}
      </p>
    </div>
  );
}

function Linked({ slug, status, onChangeRepo }: { slug: string; status: GithubStatus; onChangeRepo: () => void }) {
  const t = useTranslations("github");
  const message = useErrorMessage();
  const qc = useQueryClient();
  const link = status.link!;
  const job = status.job;
  const run = useMutation({
    mutationFn: () => githubApi.run(slug),
    onSuccess: (job) => qc.setQueryData<GithubStatus>(ghKey.status(slug), (s) => (s ? { ...s, job } : s)),
  });
  const repoLine = (
    <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
      <GitHubMark className="size-3.5" />
      <span dir="ltr" className="font-mono text-foreground">
        {link.repo}
      </span>
      <span aria-hidden>·</span>
      <GitBranch className="size-3.5" aria-hidden />
      <span dir="ltr" className="font-mono">
        {link.branch}
      </span>
    </p>
  );
  const secondary = (
    <>
      <Button variant="ghost" size="sm" onClick={() => run.mutate()} disabled={run.isPending || active(job)}>
        {run.isPending ? <Loader2 className="animate-spin" /> : <RotateCw />}
        {t("linked.rerun")}
      </Button>
      <Button variant="ghost" size="sm" onClick={onChangeRepo} disabled={active(job)}>
        {t("linked.change")}
      </Button>
    </>
  );

  let main: React.ReactNode;
  if (active(job)) {
    main = <Steps job={job!} />;
  } else if (job?.status === "failed") {
    main = (
      <div role="alert" className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm">
        <p className="flex items-center gap-2 font-medium">
          <TriangleAlert className="size-4 text-destructive" aria-hidden /> {t("linked.failed")}
        </p>
        {/HTTP 401|invalid.?api.?key|authentication_error/i.test(job.error ?? "") && (
          <p>
            {t.rich("linked.badKey", {
              link: (c) => (
                <Link href={`/app/${slug}/settings`} className="font-medium underline underline-offset-4">
                  {c}
                </Link>
              ),
            })}
          </p>
        )}
        <p className="font-mono text-xs break-words text-muted-foreground" dir="ltr">
          {job.error}
        </p>
      </div>
    );
  } else if (link.pr_url) {
    main = (
      <div className="flex flex-wrap items-center gap-3 rounded-lg border border-success/30 bg-success/8 p-3 motion-safe:animate-rise">
        <CircleCheck className="size-5 shrink-0 text-success-foreground motion-safe:animate-pop" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{t("linked.prReady", { n: link.pr_number ?? 0 })}</p>
          <p className="text-xs text-muted-foreground">{t("linked.prHint", { branch: link.branch })}</p>
        </div>
        <Button asChild>
          <a href={link.pr_url} target="_blank" rel="noreferrer">
            <GitPullRequest /> {t("linked.review")}
            <ExternalLink className="rtl:-scale-x-100" aria-hidden />
            <span className="sr-only">{t("newTab")}</span>
          </a>
        </Button>
      </div>
    );
  } else {
    main = (
      <div className="flex items-center gap-3 rounded-lg border p-3 text-sm">
        <ShieldCheck className="size-5 shrink-0 text-success-foreground" aria-hidden />
        <p>{job ? t("linked.upToDate") : t("linked.waiting")}</p>
      </div>
    );
  }
  return (
    <div className="space-y-3">
      {repoLine}
      {main}
      {run.isError && (
        <p role="alert" className="text-sm text-destructive">
          {message(run.error)}
        </p>
      )}
      <div className="flex flex-wrap gap-1">{secondary}</div>
    </div>
  );
}
