"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ExternalLink, GitPullRequest, Loader2, MessageSquarePlus, MessageSquareWarning, Sparkles, X } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, EmptyState, ErrorState } from "@/components/common";
import { VerdictBadge } from "@/components/review";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { api, qk, type FeedbackGroup, type FeedbackSettings, type FeedbackSettingsInput } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";
import { Ago } from "@/components/review/panel";
import { cn } from "@/lib/utils";

// Code labels (file type, shell command), not UI copy.
const HTML_LABEL = "HTML";
const NPM_INSTALL = "npm i @glot-i18n/feedback";

type Tab = "live" | "accepted" | "rejected";
const inTab = (g: FeedbackGroup, tab: Tab) =>
  tab === "live" ? g.status === "open" || g.status === "proposed" : g.status === tab;

/** "Reports" tab: wording problems flagged by end users through the feedback widget, grouped per key × locale. */
export function Reports({ slug, onAnnotate }: { slug: string; onAnnotate?: (g: FeedbackGroup) => void }) {
  const t = useTranslations("feedback");
  const [tab, setTab] = useState<Tab>("live");
  const q = useQuery({ queryKey: qk.feedback(slug), queryFn: () => api.feedback(slug) });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!q.data) return <Skeleton className="h-64 rounded-xl" />;
  const { settings, groups } = q.data;
  const shown = groups.filter((g) => inTab(g, tab));

  return (
    <div className="mx-auto max-w-5xl space-y-6">
      {groups.length === 0 ? (
        <EmptyState icon={MessageSquareWarning} title={t("empty.title")} body={t("empty.body")} />
      ) : (
        <>
          <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)}>
            <TabsList aria-label={t("filterLabel")}>
              {(["live", "accepted", "rejected"] as const).map((k) => (
                <TabsTrigger key={k} value={k}>
                  {t(`tabs.${k}`)} <span className="text-muted-foreground tabular-nums">{groups.filter((g) => inTab(g, k)).length}</span>
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
          {shown.length === 0 ? (
            <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">{t("none")}</p>
          ) : (
            <div className="space-y-4">
              {shown.map((g) => (
                <GroupCard key={g.id + (g.proposal ?? "")} slug={slug} group={g} settings={settings} onAnnotate={onAnnotate} />
              ))}
            </div>
          )}
        </>
      )}
      <Install publicKey={settings.public_key} />
      <SettingsForm key={JSON.stringify(settings)} slug={slug} settings={settings} />
    </div>
  );
}

/** Word-level diff (LCS) of two short strings. */
function diff(a: string, b: string): { text: string; op: "=" | "-" | "+" }[] {
  const x = a.split(/(\s+)/), y = b.split(/(\s+)/);
  const L = Array.from({ length: x.length + 1 }, () => new Array<number>(y.length + 1).fill(0));
  for (let i = x.length - 1; i >= 0; i--)
    for (let j = y.length - 1; j >= 0; j--) L[i][j] = x[i] === y[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out: { text: string; op: "=" | "-" | "+" }[] = [];
  let i = 0, j = 0;
  while (i < x.length || j < y.length) {
    if (i < x.length && j < y.length && x[i] === y[j]) {
      out.push({ text: x[i++], op: "=" });
      j++;
    } else if (i < x.length && (j >= y.length || L[i + 1][j] >= L[i][j + 1])) out.push({ text: x[i++], op: "-" });
    else out.push({ text: y[j++], op: "+" });
  }
  return out;
}

export function Diff({ before, after }: { before: string; after: string }) {
  return (
    <p className="font-mono text-sm leading-relaxed break-words whitespace-pre-wrap">
      {diff(before, after).map((p, i) =>
        p.op === "=" ? (
          <span key={i}>{p.text}</span>
        ) : p.op === "-" ? (
          <del key={i} className="bg-destructive/10 text-destructive">{p.text}</del>
        ) : (
          <ins key={i} className="bg-success/12 text-success-foreground no-underline">{p.text}</ins>
        ),
      )}
    </p>
  );
}

function GroupCard({
  slug,
  group: g,
  settings,
  onAnnotate,
}: {
  slug: string;
  group: FeedbackGroup;
  settings: FeedbackSettings;
  onAnnotate?: (g: FeedbackGroup) => void;
}) {
  const t = useTranslations("feedback");
  const qc = useQueryClient();
  const [value, setValue] = useState(g.proposal ?? "");
  const act = useMutation({
    mutationFn: (action: "propose" | "accept" | "reject" | "pr") =>
      api.feedbackAction(slug, g.id, action, action === "accept" && value !== g.proposal ? { value } : undefined),
    onSuccess: (r, action) => {
      qc.invalidateQueries({ queryKey: qk.feedback(slug) });
      if (action === "propose") setValue(r.proposal ?? "");
      if (action === "accept") {
        qc.invalidateQueries({ queryKey: qk.keys(slug) });
        toast.success(t("accepted"));
      }
      if (action === "reject") toast.success(t("rejectedToast"));
      if (action === "pr") toast.success(t("prOpened"));
    },
  });
  const busy = (a: string) => act.isPending && act.variables === a;
  const live = g.status === "open" || g.status === "proposed";
  const canPr = !!settings.github_repo && settings.has_github_token;

  return (
    <article className="rounded-xl border" aria-labelledby={`fb-${g.id}`}>
      <header className="flex flex-wrap items-center gap-2 border-b px-4 py-3">
        <h2 id={`fb-${g.id}`} className="font-mono text-sm font-medium">{g.key}</h2>
        <Badge variant="outline">{g.locale}</Badge>
        <span className="text-xs text-muted-foreground">{t("reports", { count: g.count })}</span>
        <Badge variant="secondary" className={cn("ms-auto", g.status === "proposed" && "bg-info/12 text-info-foreground")}>
          {t(`status.${g.status}`)}
        </Badge>
      </header>
      <div className="grid gap-4 p-4 md:grid-cols-2">
        <div className="space-y-3 text-sm">
          <Field label={t("source")}>{g.source}</Field>
          <Field label={t("current")}>{g.current ?? "—"}</Field>
          <div>
            <div className="mb-1 text-xs font-medium text-muted-foreground">{t("userReports")}</div>
            <ul className="space-y-1.5">
              {g.reports.map((r, i) => (
                <li key={i} className="rounded-md bg-muted/40 px-2.5 py-1.5">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <Badge variant="outline" className="h-5 text-[11px]">{t(`kind.${r.kind}`)}</Badge>
                    <Ago date={r.created_at} />
                    {r.url && <span className="truncate">{r.url}</span>}
                  </div>
                  {r.suggestion && <p className="mt-1">{t("suggested", { text: r.suggestion })}</p>}
                </li>
              ))}
            </ul>
          </div>
        </div>
        <div className="space-y-3 text-sm">
          <div className="text-xs font-medium text-muted-foreground">{t("proposal")}</div>
          {g.proposal ? (
            <>
              <Diff before={g.previous ?? g.current ?? ""} after={g.proposal} />
              {g.back_translation && (
                <div className="rounded-md border p-2.5">
                  <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
                    {t("backTranslation")} {g.verdict && <VerdictBadge verdict={g.verdict} />}
                  </div>
                  <p>{g.back_translation}</p>
                  {g.explanation && <p className="mt-1 text-xs text-muted-foreground">{g.explanation}</p>}
                </div>
              )}
              {g.issues.length > 0 && (
                <ul className="space-y-1 text-xs">
                  {g.issues.map((i, n) => (
                    <li key={n} className={i.severity === "error" ? "text-destructive" : "text-warning-foreground"}>
                      {i.message}
                    </li>
                  ))}
                </ul>
              )}
              {live && (
                <Textarea aria-label={t("proposal")} value={value} onChange={(e) => setValue(e.target.value)} rows={2} className="font-mono" />
              )}
            </>
          ) : (
            <p className="text-muted-foreground">{t("noProposal", { threshold: settings.threshold })}</p>
          )}
          <div className="flex flex-wrap gap-2">
            {live && (
              <Button size="sm" variant="outline" disabled={act.isPending} onClick={() => act.mutate("propose")}>
                {busy("propose") ? <Loader2 className="animate-spin" /> : <Sparkles />} {t(g.proposal ? "regenerate" : "propose")}
              </Button>
            )}
            {live && g.proposal && (
              <Button size="sm" disabled={act.isPending || !value.trim()} onClick={() => act.mutate("accept")}>
                {busy("accept") ? <Loader2 className="animate-spin" /> : <Check />} {t("accept")}
              </Button>
            )}
            {live && onAnnotate && (
              <Button size="sm" variant="outline" onClick={() => onAnnotate(g)}>
                <MessageSquarePlus /> {t("toAnnotation")}
              </Button>
            )}
            {live && (
              <Button size="sm" variant="ghost" disabled={act.isPending} onClick={() => act.mutate("reject")}>
                <X /> {t("reject")}
              </Button>
            )}
            {g.status === "accepted" &&
              (g.pr_url ? (
                <Button size="sm" variant="outline" asChild>
                  <a href={g.pr_url} target="_blank" rel="noreferrer">
                    <ExternalLink /> {t("viewPr")}
                  </a>
                </Button>
              ) : canPr ? (
                <Button size="sm" variant="outline" disabled={act.isPending} onClick={() => act.mutate("pr")}>
                  {busy("pr") ? <Loader2 className="animate-spin" /> : <GitPullRequest />} {t("openPr")}
                </Button>
              ) : (
                <p className="text-xs text-muted-foreground">{t("prNeedsSettings")}</p>
              ))}
          </div>
        </div>
      </div>
    </article>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-0.5 text-xs font-medium text-muted-foreground">{label}</div>
      <p className="font-mono break-words">{children}</p>
    </div>
  );
}

export function Install({ publicKey }: { publicKey: string }) {
  const t = useTranslations("feedback.install");
  const origin = useOrigin() ?? "https://glot.example.com";
  const script = `<script src="https://unpkg.com/@glot-i18n/feedback/dist/feedback.global.js"
  data-key="${publicKey}" data-endpoint="${origin}" defer></script>`;
  const npm = `import { init } from "@glot-i18n/feedback";

init({ key: "${publicKey}", endpoint: "${origin}", locale, labels: { report: "Improve translation" } });`;
  return (
    <section className="space-y-3 rounded-xl border p-4">
      <div>
        <h2 className="font-medium">{t("title")}</h2>
        <p className="text-sm text-muted-foreground">{t("body")}</p>
      </div>
      <CodeBlock code={script} label={HTML_LABEL} />
      <CodeBlock code={npm} label={NPM_INSTALL} />
    </section>
  );
}

function SettingsForm({ slug, settings: s }: { slug: string; settings: FeedbackSettings }) {
  const t = useTranslations("feedback.settings");
  const qc = useQueryClient();
  const [f, setF] = useState({
    threshold: String(s.threshold),
    github_repo: s.github_repo ?? "",
    github_base: s.github_base,
    catalog_path: s.catalog_path,
    lock_path: s.lock_path,
    github_token: "",
  });
  const save = useMutation({
    mutationFn: (b: FeedbackSettingsInput) => api.updateFeedbackSettings(slug, b),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.feedback(slug) });
      toast.success(t("saved"));
    },
  });
  const field = (k: keyof typeof f, label: string, props: React.ComponentProps<typeof Input> = {}, hint?: string) => (
    <div className="space-y-1.5">
      <Label htmlFor={`fb-${k}`}>{label}</Label>
      <Input
        id={`fb-${k}`}
        value={f[k]}
        onChange={(e) => setF({ ...f, [k]: e.target.value })}
        aria-describedby={hint ? `fb-${k}-hint` : undefined}
        {...props}
      />
      {hint && (
        <p id={`fb-${k}-hint`} className="text-xs text-muted-foreground">
          {hint}
        </p>
      )}
    </div>
  );
  return (
    <form
      className="space-y-4 rounded-xl border p-4"
      onSubmit={(e) => {
        e.preventDefault();
        const { github_token, threshold, ...rest } = f;
        save.mutate({ ...rest, threshold: Number(threshold), ...(github_token ? { github_token } : {}) });
      }}
    >
      <h2 className="font-medium">{t("title")}</h2>
      <div className="grid gap-4 sm:grid-cols-2">
        {field("threshold", t("threshold"), { type: "number", min: 1, max: 1000, required: true })}
        {field("github_repo", t("repo"), { placeholder: "acme/web" })}
        {field("github_base", t("base"))}
        {field("catalog_path", t("catalog"), { dir: "ltr", className: "font-mono" }, t("catalogHint", { locale: "{locale}" }))}
        {field("lock_path", t("lock"), { dir: "ltr", className: "font-mono" }, t("lockHint"))}
        <div className="space-y-1.5">
          {field("github_token", t("token"), {
            type: "password",
            autoComplete: "off",
            placeholder: s.has_github_token ? t("tokenStored") : "github_pat_…",
            "aria-describedby": "fb-token-hint",
          })}
          <p id="fb-token-hint" className="text-xs text-muted-foreground">{t("tokenHint")}</p>
        </div>
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={save.isPending}>
          {save.isPending && <Loader2 className="animate-spin" />} {t("save")}
        </Button>
        {s.has_github_token && (
          <Button type="button" variant="ghost" disabled={save.isPending} onClick={() => save.mutate({ github_token: "" })}>
            {t("clearToken")}
          </Button>
        )}
      </div>
    </form>
  );
}
