"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Archive,
  ArchiveRestore,
  CircleCheck,
  CircleAlert,
  CirclePause,
  CirclePlay,
  Copy,
  FileCode2,
  Loader2,
  Lock,
  MoreHorizontal,
  Pencil,
  Rocket,
  Square,
  TextCursorInput,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { useLocaleName } from "@/components/common";
import { DurationEstimate } from "./duration";
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
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Link, useRouter } from "@/i18n/navigation";
import { api, ApiError, qk, type XpAction, type XpBlocker, type XpExperiment, type XpReadiness } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useArmLabel } from "./results/shared";

const LIVE = new Set(["running", "paused", "personalized"]);
export const isLive = (e: XpExperiment) => LIVE.has(e.status);
export const isEnded = (e: XpExperiment) => e.status === "stopped" || e.status === "promoted";
/** Server rule (api.rs `delete`): drafts, or archived experiments (their results were kept on purpose until then). */
export const canDelete = (e: XpExperiment) => e.status === "draft" || !!e.archived_at;

/** Every lifecycle call, with the cache updates and toasts. Errors toast globally (providers.tsx). */
export function useXpLifecycle(slug: string) {
  const t = useTranslations("xpLifecycle");
  const qc = useQueryClient();
  const router = useRouter();
  const done = (exp?: XpExperiment) => {
    if (exp) qc.setQueryData(qk.xp(slug, exp.id), exp);
    qc.invalidateQueries({ queryKey: qk.xpList(slug) });
  };
  const act = useMutation({
    mutationFn: ({ id, ...a }: XpAction & { id: string }) => api.xpAction(slug, id, a as XpAction),
    onSuccess: (exp, a) => {
      done(exp);
      if (a.action === "promote") qc.invalidateQueries({ queryKey: qk.keys(slug) });
      toast.success(t(`toast.${a.action}`));
    },
  });
  const rename = useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) => api.xpUpdate(slug, id, { name }),
    onSuccess: (exp) => {
      done(exp);
      toast.success(t("toast.renamed"));
    },
  });
  const duplicate = useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) => api.xpDuplicate(slug, id, name),
    onSuccess: (exp) => {
      done(exp);
      toast.success(t("toast.duplicated"));
      router.push(`/app/${slug}/experiments/${exp.id}`);
    },
  });
  const remove = useMutation({
    mutationFn: ({ id }: { id: string; next?: string }) => api.xpDelete(slug, id),
    onSuccess: (_, v) => {
      qc.removeQueries({ queryKey: qk.xp(slug, v.id) });
      done();
      toast.success(t("toast.deleted"));
      if (v.next) router.push(v.next);
    },
  });
  const busy = act.isPending || rename.isPending || duplicate.isPending || remove.isPending;
  return { act, rename, duplicate, remove, busy };
}

type Dialogs = "rename" | "stop" | "delete" | null;

/**
 * Row / header "More" menu: the operations allowed in the experiment's state, nothing greyed out without a reason.
 * `detail` = rendered on the experiment page (leaving it after a delete; Edit/Launch are buttons there).
 */
export function XpActionsMenu({ exp, slug, detail = false }: { exp: XpExperiment; slug: string; detail?: boolean }) {
  const t = useTranslations("xpLifecycle");
  const life = useXpLifecycle(slug);
  const [open, setOpen] = useState<Dialogs>(null);
  const page = `/app/${slug}/experiments/${exp.id}`;
  const copyName = t("copyName", { name: exp.name }).slice(0, 100);
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={t("more", { name: exp.name })} disabled={life.busy}>
            {life.busy ? <Loader2 className="animate-spin" /> : <MoreHorizontal />}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-52">
          {exp.status === "draft" && !detail && (
            <>
              <DropdownMenuItem asChild>
                <Link href={`/app/${slug}/experiments/new?edit=${exp.id}`}>
                  <Pencil /> {t("edit")}
                </Link>
              </DropdownMenuItem>
              <DropdownMenuItem asChild>
                <Link href={`${page}#launch`}>
                  <Rocket /> {t("launch")}
                </Link>
              </DropdownMenuItem>
            </>
          )}
          {exp.status === "running" && (
            <DropdownMenuItem onSelect={() => life.act.mutate({ id: exp.id, action: "pause" })}>
              <CirclePause /> {t("pause")}
            </DropdownMenuItem>
          )}
          {exp.status === "paused" && (
            <DropdownMenuItem onSelect={() => life.act.mutate({ id: exp.id, action: "resume" })}>
              <CirclePlay /> {t("resume")}
            </DropdownMenuItem>
          )}
          {isLive(exp) && (
            <DropdownMenuItem onSelect={() => setOpen("stop")}>
              <Square /> {t("stop")}
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => setOpen("rename")}>
            <TextCursorInput /> {t("rename")}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => life.duplicate.mutate({ id: exp.id, name: copyName })}>
            <Copy /> {exp.status === "draft" ? t("duplicate") : t("duplicateDraft")}
          </DropdownMenuItem>
          {isEnded(exp) &&
            (exp.archived_at ? (
              <DropdownMenuItem onSelect={() => life.act.mutate({ id: exp.id, action: "unarchive" })}>
                <ArchiveRestore /> {t("unarchive")}
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem onSelect={() => life.act.mutate({ id: exp.id, action: "archive" })}>
                <Archive /> {t("archive")}
              </DropdownMenuItem>
            ))}
          {canDelete(exp) && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem variant="destructive" onSelect={() => setOpen("delete")}>
                <Trash2 /> {t("delete")}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <RenameDialog exp={exp} open={open === "rename"} onOpenChange={(o) => setOpen(o ? "rename" : null)} onSave={(name) => life.rename.mutate({ id: exp.id, name })} />
      <StopDialog exp={exp} open={open === "stop"} onOpenChange={(o) => setOpen(o ? "stop" : null)} onConfirm={(a) => life.act.mutate({ id: exp.id, ...a })} />
      <AlertDialog open={open === "delete"} onOpenChange={(o) => setOpen(o ? "delete" : null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="text-balance">{t("confirmDelete.title", { name: exp.name })}</AlertDialogTitle>
            <AlertDialogDescription className="text-pretty">
              {exp.status === "draft" ? t("confirmDelete.draft") : t("confirmDelete.archived")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => life.remove.mutate({ id: exp.id, next: detail ? `/app/${slug}/experiments` : undefined })}>
              {t("confirmDelete.cta")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function RenameDialog({
  exp,
  open,
  onOpenChange,
  onSave,
}: {
  exp: XpExperiment;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onSave: (name: string) => void;
}) {
  const t = useTranslations("xpLifecycle");
  const [name, setName] = useState(exp.name);
  const valid = name.trim().length > 0 && name.trim().length <= 100;
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (o) setName(exp.name);
        onOpenChange(o);
      }}
    >
      <DialogContent>
        <form
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (!valid) return;
            onSave(name.trim());
            onOpenChange(false);
          }}
        >
          <DialogHeader>
            <DialogTitle>{t("renameTitle")}</DialogTitle>
            <DialogDescription>{t("renameBody")}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-1.5">
            <Label htmlFor="xp-rename">{t("nameLabel")}</Label>
            <Input id="xp-rename" value={name} onChange={(e) => setName(e.target.value)} maxLength={100} autoFocus />
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t("cancel")}
            </Button>
            <Button type="submit" disabled={!valid}>
              {t("save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Stop, optionally publishing one variant as the official text everywhere (the existing promote route). */
export function StopDialog({
  exp,
  open,
  onOpenChange,
  onConfirm,
}: {
  exp: XpExperiment;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onConfirm: (a: XpAction) => void;
}) {
  const t = useTranslations("xpLifecycle");
  const label = useArmLabel(exp);
  const [arm, setArm] = useState(0);
  const challengers = exp.arms.filter((a) => a.arm > 0 && a.approved);
  return (
    <AlertDialog
      open={open}
      onOpenChange={(o) => {
        if (o) setArm(0);
        onOpenChange(o);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="text-balance">{t("confirmStop.title", { name: exp.name })}</AlertDialogTitle>
          <AlertDialogDescription className="text-pretty">{t("confirmStop.body")}</AlertDialogDescription>
        </AlertDialogHeader>
        <fieldset className="space-y-2">
          <legend className="mb-1 text-sm font-medium">{t("confirmStop.then")}</legend>
          {[0, ...challengers.map((a) => a.arm)].map((n) => (
            <label
              key={n}
              className={cn(
                "flex cursor-pointer gap-3 rounded-lg border p-3 text-sm transition-colors duration-150 hover:bg-muted/50 has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring",
                arm === n && "border-brand bg-brand/5",
              )}
            >
              <input type="radio" name="xp-stop" checked={arm === n} onChange={() => setArm(n)} className="mt-0.5 size-4 accent-(--brand)" />
              <span>
                <span className="block font-medium">{n === 0 ? t("confirmStop.keep") : t("confirmStop.publish", { arm: label(n) })}</span>
                <span className="block text-xs text-pretty text-muted-foreground">{n === 0 ? t("confirmStop.keepHint") : t("confirmStop.publishHint")}</span>
              </span>
            </label>
          ))}
        </fieldset>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
          <AlertDialogAction onClick={() => onConfirm(arm === 0 ? { action: "stop" } : { action: "promote", scope: "all", arm })}>
            {arm === 0 ? t("confirmStop.cta") : t("confirmStop.ctaPublish")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

export function StopButton({ exp, slug }: { exp: XpExperiment; slug: string }) {
  const t = useTranslations("xpLifecycle");
  const life = useXpLifecycle(slug);
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="ghost" size="sm" disabled={life.busy} onClick={() => setOpen(true)}>
        <Square aria-hidden /> {t("stop")}
      </Button>
      <StopDialog exp={exp} open={open} onOpenChange={setOpen} onConfirm={(a) => life.act.mutate({ id: exp.id, ...a })} />
    </>
  );
}

/** One line per blocker, in words: which variant, which key, which language. */
function useBlockerText(exp: XpExperiment) {
  const t = useTranslations("xpLifecycle.blocker");
  const label = useArmLabel(exp);
  const name = useLocaleName();
  return (b: XpBlocker) =>
    t(b.code, { arm: b.arm != null ? label(b.arm) : "", key: b.key ?? "", locale: b.locale ? name(b.locale) : "" });
}

/**
 * Draft page: Edit / Launch, and the pre-flight checklist that says exactly what is missing (server readiness codes
 * + site signals from insights). Signals only warn: a site can start sending events after launch.
 */
export function DraftPanel({ exp, slug }: { exp: XpExperiment; slug: string }) {
  const t = useTranslations("xpLifecycle");
  const qc = useQueryClient();
  const text = useBlockerText(exp);
  const check = useQuery({ queryKey: qk.xpCheck(slug, exp.id), queryFn: () => api.xpCheck(slug, exp.id) });
  const insights = useQuery({ queryKey: [...qk.insights(slug), 30], queryFn: () => api.insights(slug, { days: 30 }), retry: false });
  const [confirm, setConfirm] = useState(false);
  const start = useMutation({
    mutationFn: () => api.xpStart(slug, exp.id),
    meta: { silent: true },
    onSuccess: (x) => {
      qc.setQueryData(qk.xp(slug, exp.id), x);
      qc.invalidateQueries({ queryKey: qk.xpList(slug) });
      toast.success(t("toast.launched"));
    },
    onError: (e) => {
      // 422 = not ready: the checklist below is refreshed and says why; anything else (plan, conflict) toasts.
      qc.invalidateQueries({ queryKey: qk.xpCheck(slug, exp.id) });
      if (!(e instanceof ApiError && e.status === 422)) toast.error(e instanceof ApiError ? e.message : t("launchFailed"));
    },
  });

  const r: XpReadiness | undefined = check.data;
  const blockers = r?.blockers ?? [];
  const has = (...codes: XpBlocker["code"][]) => blockers.filter((b) => codes.includes(b.code));
  const challengers = exp.arms.filter((a) => a.arm > 0);
  const visits = insights.data?.total_visits ?? 0;
  const goalSeen = !!insights.data?.events.includes(exp.goal);
  type Item = { ok: boolean; warn?: boolean; label: string; detail?: string[] };
  const items: Item[] = r
    ? [
        { ok: !has("no_keys", "key_missing", "key_live").length, label: t("check.keys", { n: exp.keys.length }), detail: has("no_keys", "key_missing", "key_live").map(text) },
        { ok: !has("no_locales").length, label: t("check.locales", { n: exp.locales.length }), detail: has("no_locales").map(text) },
        {
          ok: !has("no_variant").length,
          label: challengers.length ? t("check.variants", { n: challengers.length }) : t("check.noVariant"),
          detail: challengers.some((a) => !a.approved) ? [t("check.unapproved", { n: challengers.filter((a) => !a.approved).length })] : undefined,
        },
        { ok: !has("cell_missing", "cell_invalid").length, label: t("check.texts"), detail: has("cell_missing", "cell_invalid").map(text) },
        ...(has("currency", "weights", "too_many_segments").length
          ? [{ ok: false, label: t("check.settings"), detail: has("currency", "weights", "too_many_segments").map(text) }]
          : []),
        { ok: visits > 0, warn: true, label: visits > 0 ? t("check.site") : t("check.noSite") },
        { ok: goalSeen, warn: true, label: goalSeen ? t("check.goal", { goal: exp.goal }) : t("check.noGoal", { goal: exp.goal }) },
      ]
    : [];
  const blocking = items.filter((i) => !i.ok && !i.warn).length;
  const warnings = items.filter((i) => !i.ok && i.warn);
  const go = () => (warnings.length ? setConfirm(true) : start.mutate());

  return (
    <section id="launch" aria-labelledby="xp-launch-title" className="scroll-mt-4 space-y-4 rounded-xl border bg-card p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 id="xp-launch-title" className="font-medium">
            {t("preflight")}
          </h2>
          <p className="mt-0.5 text-sm text-pretty text-muted-foreground" aria-live="polite">
            {!r ? t("checking") : blocking ? t("blocking", { n: blocking }) : warnings.length ? t("readyWithWarnings") : t("ready")}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" asChild>
            <Link href={`/app/${slug}/experiments/new?edit=${exp.id}`}>
              <Pencil aria-hidden /> {t("edit")}
            </Link>
          </Button>
          <Button onClick={go} disabled={!r || blocking > 0 || start.isPending} aria-describedby="xp-checklist">
            {start.isPending ? <Loader2 className="animate-spin" aria-hidden /> : <Rocket aria-hidden />} {t("launch")}
          </Button>
        </div>
      </div>

      {check.isPending ? (
        <div className="h-40 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />
      ) : (
        <ul id="xp-checklist" className="divide-y rounded-lg border">
          {items.map((i) => (
            <li key={i.label} className="flex gap-3 p-3 text-sm">
              {i.ok ? (
                <CircleCheck className="mt-0.5 size-4 shrink-0 text-success-foreground" aria-hidden />
              ) : i.warn ? (
                <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning-foreground" aria-hidden />
              ) : (
                <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden />
              )}
              <div className="min-w-0">
                <p>
                  <span className="sr-only">{i.ok ? t("ok") : i.warn ? t("warning") : t("missing")} </span>
                  {i.label}
                </p>
                {i.detail && i.detail.length > 0 && (
                  <ul className="mt-1 list-disc space-y-0.5 ps-4 text-xs text-muted-foreground">
                    {i.detail.slice(0, 12).map((d) => (
                      <li key={d} className="break-words">
                        {d}
                      </li>
                    ))}
                    {i.detail.length > 12 && <li>{t("andMore", { n: i.detail.length - 12 })}</li>}
                  </ul>
                )}
                {!i.ok && i.warn && <p className="mt-0.5 text-xs text-pretty text-muted-foreground">{t("warnHint")}</p>}
              </div>
            </li>
          ))}
        </ul>
      )}

      <DurationEstimate
        slug={slug}
        segments={exp.cells.map((c) => ({ name: c.name, rule: c.rule }))}
        arms={exp.arms.filter((a) => a.approved).length}
        goal={exp.goal}
        minN={exp.min_n}
        trafficPct={exp.traffic_pct}
      />

      <p className="flex items-start gap-2 text-xs text-pretty text-muted-foreground">
        <FileCode2 className="mt-0.5 size-3.5 shrink-0" aria-hidden />
        <span>
          {t.rich("toml", {
            file: () => <code translate="no">glot.experiments.toml</code>,
            push: () => <code translate="no">glot experiments push</code>,
            pull: () => <code translate="no">glot experiments pull</code>,
          })}
        </span>
      </p>

      <AlertDialog open={confirm} onOpenChange={setConfirm}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="text-balance">{t("confirmLaunch.title", { name: exp.name })}</AlertDialogTitle>
            <AlertDialogDescription asChild>
              <div className="space-y-2 text-pretty">
                <p>{t("confirmLaunch.body")}</p>
                <ul className="list-disc space-y-0.5 ps-4">
                  {warnings.map((w) => (
                    <li key={w.label}>{w.label}</li>
                  ))}
                </ul>
              </div>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => start.mutate()}>{t("confirmLaunch.cta")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

/** Running / paused / personalized: only the name changes; why the rest is locked, and the way out. */
export function LockedNote({ exp, slug }: { exp: XpExperiment; slug: string }) {
  const t = useTranslations("xpLifecycle");
  const life = useXpLifecycle(slug);
  return (
    <div className="flex flex-wrap items-start gap-3 rounded-xl border bg-muted/30 p-4 text-sm">
      <Lock className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
      <p className="min-w-0 flex-1 text-pretty text-muted-foreground">{t("locked")}</p>
      <Button variant="outline" size="sm" disabled={life.busy} onClick={() => life.duplicate.mutate({ id: exp.id, name: t("copyName", { name: exp.name }).slice(0, 100) })}>
        <Copy aria-hidden /> {t("duplicateDraft")}
      </Button>
    </div>
  );
}

/** Stopped / promoted: archived or not, and what delete needs. */
export function EndedNote({ exp, slug }: { exp: XpExperiment; slug: string }) {
  const t = useTranslations("xpLifecycle");
  const life = useXpLifecycle(slug);
  const archived = !!exp.archived_at;
  return (
    <div className="flex flex-wrap items-start gap-3 rounded-xl border bg-muted/30 p-4 text-sm">
      <Archive className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
      <p className="min-w-0 flex-1 text-pretty text-muted-foreground">{archived ? t("archivedNote") : t("endedNote")}</p>
      <Button
        variant="outline"
        size="sm"
        disabled={life.busy}
        onClick={() => life.act.mutate({ id: exp.id, action: archived ? "unarchive" : "archive" })}
      >
        {archived ? <ArchiveRestore aria-hidden /> : <Archive aria-hidden />} {archived ? t("unarchive") : t("archive")}
      </Button>
    </div>
  );
}
