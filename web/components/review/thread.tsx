"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowLeft,
  BadgeCheck,
  Check,
  CircleCheck,
  Eye,
  GitPullRequest,
  Heart,
  KeyRound,
  Loader2,
  MoreHorizontal,
  Pencil,
  Send,
  Sparkles,
  ThumbsDown,
  ThumbsUp,
  Trash2,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useId, useState } from "react";
import { toast } from "sonner";
import { ErrorState } from "@/components/common";
import { Diff } from "@/components/review/reports";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  ANNOTATION_STATUSES,
  api,
  ApiError,
  qk,
  type Annotation,
  type AnnotationPatch,
  type Issue,
  type Rephrase,
  type RephraseProposal,
  type ReviewComment,
  type ReviewEmoji,
  type ReviewMember,
} from "@/lib/api";
import { Ago } from "@/components/review/panel";
import { cn } from "@/lib/utils";

// Rendered with icons, not emoji (ux-brief §6.4); `label` = message key.
const REACTIONS = [
  { emoji: "approve", label: "approve", icon: BadgeCheck },
  { emoji: "+1", label: "up", icon: ThumbsUp },
  { emoji: "-1", label: "down", icon: ThumbsDown },
  { emoji: "heart", label: "heart", icon: Heart },
  { emoji: "eyes", label: "eyes", icon: Eye },
  { emoji: "check", label: "check", icon: Check },
] as const satisfies readonly { emoji: ReviewEmoji; label: string; icon: React.ComponentType<{ className?: string }> }[];
const UNASSIGNED = "none";

/** ⌘/Ctrl+Enter submits the closest form (thread replies, composer). */
const submitOnModEnter = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    e.currentTarget.form?.requestSubmit();
  }
};

/** Mentions = members whose "@Name" appears in the text (the server filters them again). */
const mentionsIn = (body: string, members: ReviewMember[]) => members.filter((m) => body.includes(`@${m.name}`)).map((m) => m.user_id);

/** Server-rendered, sanitised markdown (contract §1: never render `body` as HTML). */
function Body({ html, className }: { html: string; className?: string }) {
  return (
    <div
      className={cn("text-sm break-words [&_a]:underline [&_a]:underline-offset-4 [&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:font-mono", className)}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

/** New annotation on a text picked in the frame (sites without the widget, or the screenshot fallback). */
export function Composer({
  text,
  annotationKey,
  pending,
  onCancel,
  onSubmit,
}: {
  text: string;
  annotationKey: string | null;
  pending: boolean;
  onCancel: () => void;
  onSubmit: (b: { body: string; suggestion?: string }) => void;
}) {
  const t = useTranslations("reviewPage.composer");
  const [body, setBody] = useState("");
  const [suggestion, setSuggestion] = useState("");
  return (
    <form
      className="space-y-4 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (body.trim()) onSubmit({ body: body.trim(), suggestion: suggestion.trim() || undefined });
      }}
    >
      <h2 className="font-medium">{t("title")}</h2>
      {text ? (
        <blockquote className="border-s-2 border-brand ps-3 text-sm text-muted-foreground">{text}</blockquote>
      ) : (
        <p className="text-sm text-muted-foreground">{t("noText")}</p>
      )}
      {annotationKey && (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <KeyRound aria-hidden className="size-3.5" />
          <code dir="ltr" className="font-mono">{annotationKey}</code>
        </p>
      )}
      <div className="space-y-1.5">
        <Label htmlFor="rv-new-body">{t("body")}</Label>
        <Textarea id="rv-new-body" autoFocus required rows={3} value={body} onChange={(e) => setBody(e.target.value)} onKeyDown={submitOnModEnter} aria-describedby="rv-new-body-hint" />
        <p id="rv-new-body-hint" className="text-xs text-muted-foreground">{t("bodyHint")}</p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="rv-new-suggestion">{t("suggestion")}</Label>
        <Input id="rv-new-suggestion" value={suggestion} onChange={(e) => setSuggestion(e.target.value)} aria-describedby="rv-new-suggestion-hint" />
        <p id="rv-new-suggestion-hint" className="text-xs text-muted-foreground">{t("suggestionHint")}</p>
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={pending || !body.trim()}>
          {pending ? <Loader2 className="animate-spin" /> : <Send />} {pending ? t("submitting") : t("submit")}
        </Button>
        <Button type="button" variant="ghost" onClick={onCancel}>
          {t("cancel")}
        </Button>
      </div>
    </form>
  );
}

export function AnnotationDetail({
  slug,
  id,
  n,
  meId,
  members,
  canPr,
  onBack,
  onEditText,
  onPreview,
  onDeleted,
}: {
  slug: string;
  id: string;
  n: number;
  meId: string | undefined;
  members: ReviewMember[];
  canPr: boolean;
  onBack: () => void;
  onEditText: (a: Annotation) => void;
  onPreview: (a: Annotation, value: string | null) => void;
  onDeleted: () => void;
}) {
  const t = useTranslations("reviewPage.detail");
  const ts = useTranslations("reviewPage.status");
  const qc = useQueryClient();
  const q = useQuery({ queryKey: qk.annotation(slug, id), queryFn: () => api.annotation(slug, id) });
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const refresh = (a?: Annotation) => {
    if (a) qc.setQueryData(qk.annotation(slug, id), (old: Annotation | undefined) => ({ ...a, comments: a.comments ?? old?.comments }));
    qc.invalidateQueries({ queryKey: qk.annotations(slug) });
  };
  const update = useMutation({
    mutationFn: (b: AnnotationPatch) => api.updateAnnotation(slug, id, b),
    onSuccess: (a, b) => {
      refresh(a);
      if (b.status) toast.success(t("statusChanged", { status: ts(b.status) }));
      setEditing(false);
    },
  });
  const del = useMutation({
    mutationFn: () => api.deleteAnnotation(slug, id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.annotations(slug) });
      toast.success(t("deleted"));
      onDeleted();
    },
  });
  const react = useMutation({
    mutationFn: (emoji: ReviewEmoji) => api.reactAnnotation(slug, id, emoji),
    onSuccess: ({ reactions }) => {
      qc.setQueryData(qk.annotation(slug, id), (old: Annotation | undefined) => old && { ...old, reactions });
      qc.invalidateQueries({ queryKey: qk.annotations(slug) });
    },
  });

  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const a = q.data;
  const mine = !!meId && a?.author.user_id === meId;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 border-b px-2 py-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="rtl:-scale-x-100" /> {t("back")}
        </Button>
        <h2 className="text-sm font-medium tabular-nums">{t("title", { n })}</h2>
        {a && mine && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" className="ms-auto" aria-label={t("more")}>
                <MoreHorizontal />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => setEditing(true)}>
                <Pencil /> {t("edit")}
              </DropdownMenuItem>
              <DropdownMenuItem variant="destructive" onSelect={() => setConfirmDelete(true)}>
                <Trash2 /> {t("delete")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {!a ? (
        <div className="space-y-3 p-4">
          <Skeleton className="h-5 w-2/3" />
          <Skeleton className="h-16" />
          <Skeleton className="h-24" />
        </div>
      ) : (
        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto p-4">
          <section className="space-y-2" aria-label={t("anchor")}>
            {a.anchor.text && <blockquote className="border-s-2 border-brand ps-3 text-sm">{a.anchor.text}</blockquote>}
            <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
              <span dir="ltr" className="font-mono">{a.route}</span>
              <span>{a.locale}</span>
              {a.key ? (
                <code dir="ltr" className="font-mono text-foreground">{a.key}</code>
              ) : (
                <span>{t("noKey")}</span>
              )}
            </p>
            <Button size="sm" variant="outline" onClick={() => onEditText(a)}>
              {a.key ? <Pencil /> : <KeyRound />} {a.key ? t("editText") : t("createKey")}
              <kbd className="ms-1 hidden rounded border px-1 font-mono text-[10px] text-muted-foreground sm:inline">E</kbd>
            </Button>
          </section>

          <section className="space-y-2">
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span className="font-medium text-foreground">{a.author.name}</span>
              <Ago date={a.created_at} />
            </div>
            {editing ? (
              <EditBody initial={a.body} pending={update.isPending} onCancel={() => setEditing(false)} onSave={(body) => update.mutate({ body })} />
            ) : (
              <Body html={a.body_html} />
            )}
            <div role="group" aria-label={t("reactions")} className="flex flex-wrap gap-1">
              {REACTIONS.map(({ emoji, label, icon: Icon }) => {
                const r = a.reactions.find((x) => x.emoji === emoji);
                return (
                  <Button
                    key={emoji}
                    size="xs"
                    variant={r?.mine ? "secondary" : "ghost"}
                    aria-pressed={!!r?.mine}
                    aria-label={t(`reaction.${label}`, { count: r?.count ?? 0 })}
                    disabled={react.isPending}
                    onClick={() => react.mutate(emoji)}
                    className={cn("h-7 min-w-7", r?.mine && "text-brand")}
                  >
                    <Icon aria-hidden />
                    {!!r?.count && <span className="tabular-nums">{r.count}</span>}
                  </Button>
                );
              })}
            </div>
          </section>

          <section className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="rv-status">{t("status")}</Label>
              <Select value={a.status} onValueChange={(v) => update.mutate({ status: v as Annotation["status"] })}>
                <SelectTrigger id="rv-status" size="sm" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {ANNOTATION_STATUSES.map((s) => (
                    <SelectItem key={s} value={s}>
                      {ts(s)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="rv-assignee">{t("assignee")}</Label>
              <Select value={a.assignee?.user_id ?? UNASSIGNED} onValueChange={(v) => update.mutate({ assignee_id: v === UNASSIGNED ? null : v })}>
                <SelectTrigger id="rv-assignee" size="sm" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={UNASSIGNED}>{t("unassigned")}</SelectItem>
                  {members.map((m) => (
                    <SelectItem key={m.user_id} value={m.user_id}>
                      {m.name}
                      {m.user_id === meId ? ` ${t("you")}` : ""}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </section>

          {a.status !== "resolved" && (
            <Button variant="outline" className="w-full" disabled={update.isPending} onClick={() => update.mutate({ status: "resolved" })}>
              <CircleCheck /> {t("resolve")}
              <kbd className="ms-1 hidden rounded border px-1 font-mono text-[10px] text-muted-foreground sm:inline">R</kbd>
            </Button>
          )}

          <Wording slug={slug} a={a} canPr={canPr} onPreview={onPreview} onEditText={onEditText} onApplied={refresh} />

          <Thread slug={slug} a={a} meId={meId} members={members} />
        </div>
      )}
      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("deleteTitle", { n })}</AlertDialogTitle>
            <AlertDialogDescription>{t("deleteBody", { count: a?.comment_count ?? 0 })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("keep")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => del.mutate()}>
              {t("deleteConfirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function EditBody({ initial, pending, onSave, onCancel }: { initial: string; pending: boolean; onSave: (b: string) => void; onCancel: () => void }) {
  const t = useTranslations("reviewPage.detail");
  const id = useId();
  const [v, setV] = useState(initial);
  return (
    <form
      className="space-y-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (v.trim()) onSave(v.trim());
      }}
    >
      <Label htmlFor={id} className="sr-only">
        {t("edit")}
      </Label>
      <Textarea id={id} autoFocus rows={3} value={v} onChange={(e) => setV(e.target.value)} onKeyDown={submitOnModEnter} />
      <div className="flex gap-2">
        <Button size="sm" type="submit" disabled={pending || !v.trim()}>
          {pending && <Loader2 className="animate-spin" />} {t("save")}
        </Button>
        <Button size="sm" type="button" variant="ghost" onClick={onCancel}>
          {t("cancel")}
        </Button>
      </div>
    </form>
  );
}

/** Suggestion → AI rewordings (RV4) → apply as the reviewed translation, previewed live in the frame. */
function Wording({
  slug,
  a,
  canPr,
  onPreview,
  onEditText,
  onApplied,
}: {
  slug: string;
  a: Annotation;
  canPr: boolean;
  onPreview: (a: Annotation, value: string | null) => void;
  onEditText: (a: Annotation) => void;
  onApplied: (a: Annotation) => void;
}) {
  const t = useTranslations("reviewPage.wording");
  const qc = useQueryClient();
  const current = a.translation?.value ?? a.anchor.text;
  const [value, setValue] = useState(a.suggestion ?? "");
  const [instruction, setInstruction] = useState("");
  const [rewrite, setRewrite] = useState<Rephrase | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [issues, setIssues] = useState<Issue[]>([]);
  const [applied, setApplied] = useState(false);

  // A new suggestion from the server (RV4 or another reviewer) replaces the draft.
  const [seen, setSeen] = useState(a.suggestion);
  if (seen !== a.suggestion) {
    setSeen(a.suggestion);
    setValue(a.suggestion ?? "");
  }
  const previewing = value.trim() && value !== current ? value : null;
  useEffect(() => {
    onPreview(a, previewing);
  }, [previewing]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => onPreview(a, null), []); // eslint-disable-line react-hooks/exhaustive-deps

  const textOf = (p: RephraseProposal) => p.variants[a.locale]?.value ?? p.text;
  const proposal = rewrite?.proposals.find((p) => p.id === chosen);
  // Applying an untouched proposal writes all its locales at once (RV4 /rephrase/apply); an edited text only this locale.
  const whole = !!proposal && proposal.valid && value === textOf(proposal);
  const rephrase = useMutation({
    mutationFn: () => api.rephraseAnnotation(slug, a.id, { instruction: instruction.trim() || undefined }),
    onSuccess: (r) => {
      setRewrite(r);
      const first = r.proposals.find((p) => p.valid);
      setChosen(first?.id ?? null);
      if (first) setValue(textOf(first));
      qc.invalidateQueries({ queryKey: qk.annotation(slug, a.id) });
    },
  });
  const apply = useMutation({
    meta: { silent: true },
    mutationFn: async () =>
      whole ? (await api.applyRephrase(slug, a.id, { rewrite_id: rewrite!.rewrite_id, proposal_id: proposal!.id })).annotation : api.applyAnnotation(slug, a.id, { value: value.trim() }),
    onSuccess: (r) => {
      setIssues([]);
      setApplied(true);
      onApplied(r);
      qc.invalidateQueries({ queryKey: qk.keys(slug) });
      if (a.key) qc.invalidateQueries({ queryKey: qk.text(slug, a.key) });
      toast.success(t("applied", { locale: a.locale }));
    },
    onError: (e) => {
      if (e instanceof ApiError && e.code === "invalid_translation") {
        const d = e.details as { results?: Record<string, { issues: Issue[] }> } | undefined;
        setIssues(
          d?.results
            ? Object.entries(d.results).flatMap(([l, x]) => x.issues.map((i) => ({ ...i, message: l === a.locale ? i.message : `${l}: ${i.message}` })))
            : [{ code: e.code, severity: "error", message: e.message }],
        );
      } else toast.error(e instanceof ApiError ? e.message : t("failed"));
    },
  });
  const pr = useMutation({
    mutationFn: () => api.feedbackAction(slug, a.feedback_group_id!, "pr"),
    onSuccess: (g) => {
      toast.success(t("prOpened"));
      if (g.pr_url) window.open(g.pr_url, "_blank", "noopener,noreferrer");
    },
  });

  if (!a.key)
    return (
      <section className="space-y-2 rounded-lg border border-dashed p-3 text-sm">
        <h3 className="font-medium">{t("title")}</h3>
        <p className="text-muted-foreground">{t("noKey")}</p>
        <Button size="sm" variant="outline" onClick={() => onEditText(a)}>
          <KeyRound /> {t("createKey")}
        </Button>
      </section>
    );

  return (
    <section className="space-y-3 rounded-lg border p-3" aria-labelledby={`rv-wording-${a.id}`}>
      <div className="flex items-center gap-2">
        <h3 id={`rv-wording-${a.id}`} className="text-sm font-medium">
          {t("title")}
        </h3>
        <span className="text-xs text-muted-foreground">{a.locale}</span>
      </div>
      {a.source && (
        <p className="text-xs text-muted-foreground">
          {t("source")} <span className="text-foreground">{a.source}</span>
        </p>
      )}
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          rephrase.mutate();
        }}
      >
        <Label htmlFor={`rv-instr-${a.id}`} className="sr-only">
          {t("instruction")}
        </Label>
        <Input
          id={`rv-instr-${a.id}`}
          value={instruction}
          onChange={(e) => setInstruction(e.target.value)}
          placeholder={t("instructionPlaceholder")}
          className="h-8 flex-1 text-sm"
        />
        <Button type="submit" size="sm" variant="secondary" disabled={rephrase.isPending} aria-busy={rephrase.isPending}>
          {rephrase.isPending ? <Loader2 className="animate-spin" /> : <Sparkles />}
          {rephrase.isPending ? t("rephrasing") : t("rephrase")}
        </Button>
      </form>
      {rewrite && rewrite.proposals.length > 0 && (
        <fieldset className="space-y-1.5">
          <legend className="mb-1 text-xs font-medium text-muted-foreground">{t("variants")}</legend>
          {rewrite.proposals.map((p) => {
            const locales = Object.entries(p.variants);
            return (
              <div
                key={p.id}
                className={cn(
                  "rounded-md border p-2 text-sm transition-colors has-focus-visible:ring-2 has-focus-visible:ring-ring",
                  chosen === p.id ? "border-brand bg-brand/5" : "hover:bg-accent",
                  !p.valid && "opacity-70",
                )}
                onMouseEnter={() => onPreview(a, textOf(p))}
                onMouseLeave={() => onPreview(a, previewing)}
              >
                <label className="flex cursor-pointer gap-2">
                  <input
                    type="radio"
                    name={`rv-variant-${a.id}`}
                    className="mt-1 accent-(--brand)"
                    checked={chosen === p.id}
                    disabled={!p.valid}
                    onFocus={() => onPreview(a, textOf(p))}
                    onChange={() => {
                      setChosen(p.id);
                      setValue(textOf(p));
                    }}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block">{textOf(p)}</span>
                    {p.explanation && <span className="block text-xs text-muted-foreground">{p.explanation}</span>}
                    <span className={cn("block text-xs", p.valid ? "text-success-foreground" : "text-destructive")}>
                      {p.valid ? t("allLocales", { count: locales.length }) : t("invalidProposal")}
                    </span>
                  </span>
                </label>
                {locales.length > 1 && (
                  <details className="mt-1 ps-6 text-xs">
                    <summary className="cursor-pointer rounded-sm text-muted-foreground">{t("showLocales", { count: locales.length })}</summary>
                    <dl className="mt-1 space-y-1">
                      {locales.map(([l, v]) => (
                        <div key={l}>
                          <dt className="font-mono text-muted-foreground">{l}</dt>
                          <dd lang={l} dir="auto">
                            {v.value}
                            {v.issues.map((i, k) => (
                              <span key={k} className={cn("block", i.severity === "error" ? "text-destructive" : "text-warning-foreground")}>
                                {i.message}
                              </span>
                            ))}
                          </dd>
                        </div>
                      ))}
                    </dl>
                  </details>
                )}
              </div>
            );
          })}
          {rewrite.rejected.length > 0 && <p className="text-xs text-muted-foreground">{t("rejected", { count: rewrite.rejected.length })}</p>}
        </fieldset>
      )}
      <div className="space-y-1.5">
        <Label htmlFor={`rv-value-${a.id}`}>{t("newWording")}</Label>
        <Textarea
          id={`rv-value-${a.id}`}
          rows={2}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={current}
          aria-invalid={issues.some((i) => i.severity === "error") || undefined}
          aria-describedby={`rv-value-hint-${a.id}`}
        />
        <p id={`rv-value-hint-${a.id}`} className="text-xs text-muted-foreground">
          {t("previewHint")}
        </p>
        {value.trim() && value !== current && <Diff before={current} after={value} />}
        {issues.length > 0 && (
          <ul role="alert" className="space-y-1 text-xs">
            {issues.map((i, k) => (
              <li key={k} className={i.severity === "error" ? "text-destructive" : "text-warning-foreground"}>
                {i.message}
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={apply.isPending || !value.trim() || value === current} onClick={() => apply.mutate()}>
          {apply.isPending ? <Loader2 className="animate-spin" /> : <Check />}
          {whole ? t("applyAll", { count: Object.keys(proposal!.variants).length }) : t("apply")}
        </Button>
        {a.feedback_group_id && (applied || a.status === "resolved") &&
          (canPr ? (
            <Button size="sm" variant="outline" disabled={pr.isPending} onClick={() => pr.mutate()}>
              {pr.isPending ? <Loader2 className="animate-spin" /> : <GitPullRequest />} {t("openPr")}
            </Button>
          ) : (
            <p className="text-xs text-muted-foreground">{t("prNeedsSettings")}</p>
          ))}
      </div>
    </section>
  );
}

function Thread({ slug, a, meId, members }: { slug: string; a: Annotation; meId: string | undefined; members: ReviewMember[] }) {
  const t = useTranslations("reviewPage.thread");
  const qc = useQueryClient();
  const [reply, setReply] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const done = () => {
    qc.invalidateQueries({ queryKey: qk.annotation(slug, a.id) });
    qc.invalidateQueries({ queryKey: qk.annotations(slug) });
  };
  const send = useMutation({
    mutationFn: (body: string) => api.commentAnnotation(slug, a.id, { body, mentions: mentionsIn(body, members) }),
    onSuccess: () => {
      setReply("");
      done();
    },
  });
  const edit = useMutation({
    mutationFn: ({ cid, body }: { cid: string; body: string }) => api.updateComment(slug, a.id, cid, { body }),
    onSuccess: () => {
      setEditing(null);
      done();
    },
  });
  const del = useMutation({ mutationFn: (cid: string) => api.deleteComment(slug, a.id, cid), onSuccess: done });
  const comments: ReviewComment[] = a.comments ?? [];

  return (
    <section className="space-y-3" aria-labelledby={`rv-thread-${a.id}`}>
      <h3 id={`rv-thread-${a.id}`} className="text-sm font-medium">
        {t("title", { count: comments.length })}
      </h3>
      {comments.length > 0 && (
        <ol className="space-y-3">
          {comments.map((c) => (
            <li key={c.id} className="rounded-lg bg-muted/40 p-2.5">
              <div className="mb-1 flex items-center gap-2 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{c.author.name}</span>
                <Ago date={c.created_at} />
                {c.edited_at && <span>{t("edited")}</span>}
                {meId && c.author.user_id === meId && editing !== c.id && (
                  <span className="ms-auto flex gap-0.5">
                    <Button size="icon-xs" variant="ghost" aria-label={t("editComment")} onClick={() => setEditing(c.id)}>
                      <Pencil />
                    </Button>
                    <Button size="icon-xs" variant="ghost" aria-label={t("deleteComment")} disabled={del.isPending} onClick={() => del.mutate(c.id)}>
                      <Trash2 />
                    </Button>
                  </span>
                )}
              </div>
              {editing === c.id ? (
                <EditBody initial={c.body} pending={edit.isPending} onCancel={() => setEditing(null)} onSave={(body) => edit.mutate({ cid: c.id, body })} />
              ) : (
                <Body html={c.body_html} />
              )}
            </li>
          ))}
        </ol>
      )}
      <form
        className="space-y-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (reply.trim()) send.mutate(reply.trim());
        }}
      >
        <Label htmlFor={`rv-reply-${a.id}`} className="sr-only">
          {t("reply")}
        </Label>
        <Textarea
          id={`rv-reply-${a.id}`}
          rows={2}
          value={reply}
          onChange={(e) => setReply(e.target.value)}
          onKeyDown={submitOnModEnter}
          placeholder={t("replyPlaceholder")}
          aria-describedby={`rv-reply-hint-${a.id}`}
        />
        <div className="flex items-center gap-2">
          <p id={`rv-reply-hint-${a.id}`} className="flex-1 text-xs text-muted-foreground">
            {t("replyHint")}
          </p>
          <Button type="submit" size="sm" disabled={send.isPending || !reply.trim()}>
            {send.isPending ? <Loader2 className="animate-spin" /> : <Send className="rtl:-scale-x-100" />} {t("send")}
          </Button>
        </div>
      </form>
    </section>
  );
}
