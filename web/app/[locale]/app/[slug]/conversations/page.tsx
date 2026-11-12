"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, MessagesSquare, Send, Settings2, ShieldCheck, TriangleAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { CodeBlock, CopyButton, EmptyState, ErrorState, PageHeader, useLocaleName } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Link } from "@/i18n/navigation";
import { useOrigin, useSlug } from "@/lib/hooks";
import {
  twoWay,
  twoWayKeys,
  type TwoWayConversation,
  type TwoWayMessage,
  type TwoWayMode,
  type TwoWaySettings,
  type TwoWaySettingsInput,
} from "@/lib/two-way";
import { useErrorMessage } from "@/lib/use-error-message";
import { cn } from "@/lib/utils";

const MODES: TwoWayMode[] = ["suggest", "one_click", "auto"];

/** Conversations: what customers write, in the team's language next to the original; replies go back in theirs. */
export default function ConversationsPage() {
  const t = useTranslations("twoWay");
  const slug = useSlug();
  const settings = useQuery({ queryKey: twoWayKeys.settings(slug), queryFn: () => twoWay.settings(slug), enabled: !!slug });
  const list = useQuery({
    queryKey: twoWayKeys.conversations(slug),
    queryFn: () => twoWay.conversations(slug),
    enabled: !!slug,
    refetchInterval: 15_000,
  });
  const [picked, setPicked] = useState<string | null>(null);
  const [configuring, setConfiguring] = useState(false);

  const header = (
    <PageHeader
      title={t("title")}
      subtitle={t("subtitle")}
      actions={
        <>
          {settings.data && <Badge variant="outline">{t(`mode.${settings.data.mode}.name`)}</Badge>}
          <Button variant="outline" onClick={() => setConfiguring(true)} disabled={!settings.data}>
            <Settings2 /> {t("settings.open")}
          </Button>
        </>
      }
    />
  );
  const error = settings.error ?? list.error;
  if (error) {
    return (
      <>
        {header}
        <ErrorState error={error} onRetry={() => {
            settings.refetch();
            list.refetch();
          }} />
      </>
    );
  }
  const rows = list.data?.conversations;
  const current = picked ?? rows?.[0]?.id ?? null;
  return (
    <>
      {header}
      {!rows || !settings.data ? (
        <Skeleton className="h-[50vh] rounded-xl" />
      ) : rows.length === 0 ? (
        <Connect slug={slug} settings={settings.data} />
      ) : (
        <>
          <Languages slug={slug} />
          <div className="grid gap-4 lg:grid-cols-[18rem_1fr]">
            <ul className="flex max-h-[70vh] flex-col gap-1 overflow-y-auto" aria-label={t("title")}>
              {rows.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    onClick={() => setPicked(c.id)}
                    aria-current={c.id === current ? "true" : undefined}
                    className={cn(
                      "w-full rounded-lg border px-3 py-2 text-start text-sm transition-colors hover:bg-accent",
                      c.id === current ? "border-brand/50 bg-accent" : "border-transparent",
                    )}
                  >
                    <span className="flex items-center gap-2">
                      <span className="font-mono text-xs uppercase text-muted-foreground">{c.customer_locale ?? "?"}</span>
                      <span className="truncate font-medium">{c.subject ?? c.customer_name ?? c.external_id}</span>
                      {c.attention > 0 && <TriangleAlert className="ms-auto size-3.5 shrink-0 text-warning-foreground" aria-label={t("attention")} />}
                    </span>
                    <span className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{c.preview}</span>
                    <span className="mt-1 block text-[11px] text-muted-foreground">{t(`channel.${c.channel}`)}</span>
                  </button>
                </li>
              ))}
            </ul>
            {current && <Thread key={current} slug={slug} id={current} settings={settings.data} />}
          </div>
        </>
      )}
      {settings.data && configuring && <SettingsDialog slug={slug} settings={settings.data} onClose={() => setConfiguring(false)} />}
    </>
  );
}

/** Honest empty state: no sample conversations, only how to get the first real message in. */
function Connect({ slug, settings }: { slug: string; settings: TwoWaySettings }) {
  const t = useTranslations("twoWay");
  const origin = useOrigin() ?? "https://glot.example.com";
  const curl = `curl -X POST ${origin}/api/projects/${slug}/two-way/inbound \\
  -H "Authorization: Bearer $GLOT_TOKEN" -H "Content-Type: application/json" \\
  -d '{"conversation_id": "ticket-42", "text": "注文した商品がまだ届いていません。"}'`;
  const script = `<script src="${origin}${settings.paths.form_script}" data-project="${slug}" async></script>
<form data-glot-form …>`;
  return (
    <EmptyState
      icon={MessagesSquare}
      title={t("empty.title")}
      body={t("empty.body")}
      action={
        <Button variant="outline" asChild>
          <Link href="/docs/two-way">{t("empty.docs")}</Link>
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <CodeBlock label={t("empty.api")} code={curl} />
        <CodeBlock label={settings.form_enabled ? t("empty.form") : t("empty.formOff")} code={script} />
        <p className="text-xs text-muted-foreground">{t("empty.helpdesk")}</p>
      </div>
    </EmptyState>
  );
}

/** Which languages customers write in (real counts, last 90 days). */
function Languages({ slug }: { slug: string }) {
  const t = useTranslations("twoWay");
  const localeName = useLocaleName();
  const stats = useQuery({ queryKey: twoWayKeys.stats(slug), queryFn: () => twoWay.stats(slug) });
  if (!stats.data || stats.data.total === 0) return null;
  return (
    <section className="mb-6 rounded-xl border p-4" aria-labelledby="tw-langs">
      <h2 id="tw-langs" className="text-sm font-medium">
        {t("stats.title", { count: stats.data.total, days: stats.data.days })}
      </h2>
      <ul className="mt-3 flex flex-col gap-1.5">
        {stats.data.languages.map((l) => (
          <li key={l.locale} className="grid grid-cols-[9rem_1fr_auto] items-center gap-3 text-sm">
            <span className="truncate capitalize">{l.locale === "und" ? t("stats.unknown") : localeName(l.locale)}</span>
            <span className="h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden>
              <span className="block h-full rounded-full bg-brand" style={{ width: `${Math.max(2, l.share * 100)}%` }} />
            </span>
            <span className="flex items-center gap-2 tabular-nums text-muted-foreground">
              {l.messages}
              {!l.served && l.locale !== "und" && <Badge variant="outline">{t("stats.unserved")}</Badge>}
            </span>
          </li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-muted-foreground">{t("stats.hint")}</p>
    </section>
  );
}

function Flags({ m }: { m: TwoWayMessage }) {
  const t = useTranslations("twoWay");
  const known = ["language_uncertain", "low_confidence", "pii_lost"] as const;
  return (
    <span className="flex flex-wrap items-center gap-1.5">
      {m.pii_masked > 0 && (
        <Badge variant="outline" title={t("flag.piiTip")}>
          <ShieldCheck /> {t("flag.pii", { count: m.pii_masked })}
        </Badge>
      )}
      {m.flags.map((f) => {
        const k = known.find((x) => x === f);
        const label = k ? t(`flag.${k}`) : f.startsWith("mixed:") ? t("flag.mixed", { languages: f.slice(6).replaceAll("+", ", ") }) : f.startsWith("untranslated:") ? t("flag.untranslated") : t("flag.validation");
        return (
          <Badge key={f} variant="outline" className="border-warning/50 text-warning-foreground">
            {label}
          </Badge>
        );
      })}
    </span>
  );
}

function Thread({ slug, id, settings }: { slug: string; id: string; settings: TwoWaySettings }) {
  const t = useTranslations("twoWay");
  const qc = useQueryClient();
  const message = useErrorMessage();
  const localeName = useLocaleName();
  const conv = useQuery({ queryKey: twoWayKeys.conversation(slug, id), queryFn: () => twoWay.conversation(slug, id) });
  const refresh = (c?: TwoWayConversation) => {
    if (c) qc.setQueryData(twoWayKeys.conversation(slug, id), c);
    else qc.invalidateQueries({ queryKey: twoWayKeys.conversation(slug, id) });
    qc.invalidateQueries({ queryKey: twoWayKeys.conversations(slug) });
  };
  const send = useMutation({
    mutationFn: (messageId: string) => twoWay.send(slug, messageId),
    onSuccess: (m) => {
      if (m.status === "sent") toast.success(t("reply.sent"));
      else toast.error(m.error ?? t("reply.failed"));
      refresh();
    },
    onError: (e) => toast.error(message(e)),
  });
  const [language, setLanguage] = useState("");
  const fix = useMutation({
    mutationFn: () => twoWay.setLanguage(slug, id, language.trim()),
    onSuccess: (c) => {
      setLanguage("");
      refresh(c);
    },
    onError: (e) => toast.error(message(e)),
  });

  if (conv.isError) return <ErrorState error={conv.error} onRetry={() => conv.refetch()} />;
  if (!conv.data) return <Skeleton className="h-[50vh] rounded-xl" />;
  const c = conv.data;
  const unsure = c.messages.some((m) => m.flags.includes("language_uncertain") || m.flags.includes("untranslated:no_language"));
  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
        <h2 className="font-medium">{c.subject ?? c.external_id}</h2>
        <span className="text-muted-foreground">
          {c.customer_locale ? t("thread.language", { language: localeName(c.customer_locale) }) : t("thread.languageUnknown")}
        </span>
        {(unsure || !c.customer_locale) && (
          <form
            className="flex items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (language.trim()) fix.mutate();
            }}
          >
            <Label htmlFor="tw-lang" className="sr-only">
              {t("thread.fixLabel")}
            </Label>
            <Input id="tw-lang" value={language} onChange={(e) => setLanguage(e.target.value)} placeholder="ja" className="h-8 w-20 font-mono text-xs" />
            <Button type="submit" size="sm" variant="outline" disabled={fix.isPending || !language.trim()}>
              {fix.isPending && <Loader2 className="animate-spin" />} {t("thread.fix")}
            </Button>
          </form>
        )}
      </div>

      <ol className="flex flex-col gap-3">
        {c.messages.map((m) => (
          <li key={m.id} className={cn("rounded-xl border p-3", m.direction === "out" && "bg-muted/40")}>
            <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span className="font-medium text-foreground">{t(m.direction === "in" ? "thread.customer" : "thread.team")}</span>
              <time dateTime={m.created_at}>{new Date(m.created_at).toLocaleString()}</time>
              {m.direction === "out" && <Badge variant={m.status === "sent" ? "secondary" : "outline"}>{t(`status.${m.status}`)}</Badge>}
              <Flags m={m} />
            </div>
            <div className="grid gap-3 md:grid-cols-2">
              <Side label={t("thread.original", { language: m.original_locale ?? "?" })} text={m.original} lang={m.original_locale} />
              {m.translation !== null ? (
                <Side label={t("thread.translation", { language: m.translation_locale ?? "?" })} text={m.translation} lang={m.translation_locale} copy />
              ) : (
                <p className="self-center text-xs text-muted-foreground">{m.error ?? t("thread.sameLanguage")}</p>
              )}
            </div>
            {m.direction === "out" && m.back_translation && (
              <p className="mt-2 border-t pt-2 text-xs text-muted-foreground">
                <span className="font-medium">{t("reply.back")}</span> {m.back_translation}
                {m.confidence !== null && <span className="ms-2 tabular-nums">({t("reply.confidence", { percent: Math.round(m.confidence * 100) })})</span>}
              </p>
            )}
            {m.direction === "out" && m.error && m.translation !== null && <p className="mt-2 text-xs text-destructive">{m.error}</p>}
            {m.direction === "out" && m.status !== "sent" && settings.mode !== "suggest" && (
              <Button size="sm" className="mt-2" onClick={() => send.mutate(m.id)} disabled={send.isPending}>
                <Send /> {t("reply.send")}
              </Button>
            )}
          </li>
        ))}
      </ol>

      {c.customer_locale && <ReplyBox slug={slug} id={id} mode={settings.mode} onDone={() => refresh()} />}
    </div>
  );
}

function Side({ label, text, lang, copy }: { label: string; text: string; lang: string | null; copy?: boolean }) {
  return (
    <div className="min-w-0">
      <div className="mb-1 flex items-center justify-between text-[11px] font-medium text-muted-foreground">
        <span>{label}</span>
        {copy && <CopyButton value={text} />}
      </div>
      <p lang={lang ?? undefined} dir="auto" className="text-sm break-words whitespace-pre-wrap">
        {text}
      </p>
    </div>
  );
}

/** Reply in the team's language; the back-translation of what the customer will read updates as you pause typing. */
function ReplyBox({ slug, id, mode, onDone }: { slug: string; id: string; mode: TwoWayMode; onDone: () => void }) {
  const t = useTranslations("twoWay");
  const message = useErrorMessage();
  const [text, setText] = useState("");
  const [settled, setSettled] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSettled(text.trim()), 900);
    return () => clearTimeout(timer);
  }, [text]);
  const preview = useQuery({
    queryKey: ["two-way", slug, "preview", id, settled],
    queryFn: () => twoWay.preview(slug, id, settled),
    enabled: settled.length > 1,
    staleTime: Infinity,
    retry: false,
  });
  const reply = useMutation({
    mutationFn: (send: boolean) => twoWay.reply(slug, id, text.trim(), send),
    onSuccess: (m) => {
      setText("");
      if (m.status === "sent") toast.success(t("reply.sent"));
      else if (m.status === "failed") toast.error(m.error ?? t("reply.failed"));
      else toast.success(t("reply.saved"));
      onDone();
    },
    onError: (e) => toast.error(message(e)),
  });
  const p = settled === text.trim() ? preview.data : undefined;
  return (
    <form
      className="rounded-xl border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (text.trim()) reply.mutate(false);
      }}
    >
      <Label htmlFor="tw-reply" className="text-sm font-medium">
        {t("reply.label")}
      </Label>
      <Textarea id="tw-reply" className="mt-2" rows={3} value={text} onChange={(e) => setText(e.target.value)} placeholder={t("reply.placeholder")} />
      <div className="mt-2 min-h-12 text-xs text-muted-foreground" aria-live="polite">
        {preview.isFetching ? (
          <span className="flex items-center gap-1.5">
            <Loader2 className="size-3 animate-spin" /> {t("reply.checking")}
          </span>
        ) : preview.isError ? (
          <span className="text-destructive">{message(preview.error)}</span>
        ) : p ? (
          <>
            <p dir="auto" className="text-sm text-foreground">
              {p.translation}
            </p>
            {p.back_translation && (
              <p className="mt-1">
                <span className="font-medium">{t("reply.back")}</span> {p.back_translation}{" "}
                <span className={cn("tabular-nums", p.needs_review && "text-warning-foreground")}>
                  ({t("reply.confidence", { percent: Math.round(p.confidence * 100) })})
                </span>
              </p>
            )}
            {p.needs_review && <p className="mt-1 text-warning-foreground">{t("reply.review")}</p>}
          </>
        ) : (
          t("reply.hint")
        )}
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <Button type="submit" variant={mode === "suggest" ? "default" : "outline"} disabled={!text.trim() || reply.isPending}>
          {reply.isPending && <Loader2 className="animate-spin" />} {t("reply.save")}
        </Button>
        {mode !== "suggest" && (
          <Button type="button" onClick={() => reply.mutate(true)} disabled={!text.trim() || reply.isPending}>
            <Send /> {t("reply.send")}
          </Button>
        )}
        <span className="text-xs text-muted-foreground">{t(`mode.${mode}.hint`)}</span>
      </div>
    </form>
  );
}

function SettingsDialog({ slug, settings, onClose }: { slug: string; settings: TwoWaySettings; onClose: () => void }) {
  const t = useTranslations("twoWay");
  const tc = useTranslations("common");
  const qc = useQueryClient();
  const message = useErrorMessage();
  const origin = useOrigin() ?? "";
  const [draft, setDraft] = useState<TwoWaySettingsInput>({});
  const [secret, setSecret] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: (b: TwoWaySettingsInput) => twoWay.saveSettings(slug, b),
    onSuccess: (s, b) => {
      qc.setQueryData(twoWayKeys.settings(slug), { ...s, webhook_secret: undefined });
      if (s.webhook_secret) setSecret(s.webhook_secret);
      else if (!b.generate_webhook_secret) {
        toast.success(t("settings.saved"));
        onClose();
      }
    },
    onError: (e) => toast.error(message(e)),
  });
  const set = (b: TwoWaySettingsInput) => setDraft((d) => ({ ...d, ...b }));
  const mode = draft.mode ?? settings.mode;
  const field = (id: string, label: string, input: React.ReactNode, hint?: string) => (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      {input}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("settings.title")}</DialogTitle>
        </DialogHeader>
        <form
          className="grid gap-5"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate(draft);
          }}
        >
          {field(
            "tw-mode",
            t("settings.mode"),
            <Select value={mode} onValueChange={(v) => set({ mode: v as TwoWayMode })}>
              <SelectTrigger id="tw-mode" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {MODES.map((m) => (
                  <SelectItem key={m} value={m}>
                    {t(`mode.${m}.name`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>,
            t(`mode.${mode}.hint`),
          )}
          <div className="grid gap-4 sm:grid-cols-2">
            {field(
              "tw-team",
              t("settings.team"),
              <Input id="tw-team" className="font-mono" defaultValue={settings.team_locale} onChange={(e) => set({ team_locale: e.target.value })} />,
            )}
            {field(
              "tw-review",
              t("settings.review"),
              <Input
                id="tw-review"
                type="number"
                min={0}
                max={100}
                defaultValue={Math.round(settings.review_below * 100)}
                onChange={(e) => set({ review_below: Number(e.target.value) / 100 })}
              />,
              t("settings.reviewHint"),
            )}
          </div>

          <fieldset className="grid gap-3 rounded-lg border p-3">
            <legend className="px-1 text-sm font-medium">{t("settings.webhook")}</legend>
            <code className="text-xs break-all">{origin + settings.paths.webhook}</code>
            {secret ? (
              <div className="flex items-center gap-2">
                <code className="text-xs break-all">{secret}</code>
                <CopyButton value={secret} />
              </div>
            ) : null}
            <p className="text-xs text-muted-foreground">{secret ? t("settings.secretOnce") : settings.has_webhook_secret ? t("settings.secretSet") : t("settings.secretNone")}</p>
            <Button type="button" variant="outline" size="sm" className="w-fit" disabled={save.isPending} onClick={() => save.mutate({ generate_webhook_secret: true })}>
              {settings.has_webhook_secret || secret ? t("settings.rotate") : t("settings.generate")}
            </Button>
            {field(
              "tw-reply-url",
              t("settings.replyUrl"),
              <Input id="tw-reply-url" type="url" defaultValue={settings.reply_url ?? ""} onChange={(e) => set({ reply_url: e.target.value })} placeholder="https://" />,
              t("settings.replyUrlHint"),
            )}
          </fieldset>

          <fieldset className="grid gap-3 rounded-lg border p-3">
            <legend className="px-1 text-sm font-medium">{t("settings.form")}</legend>
            <label className="flex items-center justify-between gap-3 text-sm">
              {t("settings.formEnabled")}
              <Switch checked={draft.form_enabled ?? settings.form_enabled} onCheckedChange={(v) => set({ form_enabled: v })} />
            </label>
            <label className="flex items-center justify-between gap-3 text-sm">
              {t("settings.formStore")}
              <Switch checked={draft.form_store ?? settings.form_store} onCheckedChange={(v) => set({ form_store: v })} />
            </label>
            <p className="text-xs text-muted-foreground">{t("settings.formHint")}</p>
          </fieldset>

          <fieldset className="grid gap-3 rounded-lg border p-3">
            <legend className="px-1 text-sm font-medium">Help Scout</legend>
            <code className="text-xs break-all">{origin + settings.paths.helpscout}</code>
            {field(
              "tw-hs-id",
              t("settings.hsAppId"),
              <Input id="tw-hs-id" defaultValue={settings.helpscout.app_id ?? ""} onChange={(e) => set({ helpscout_app_id: e.target.value })} autoComplete="off" />,
            )}
            {field(
              "tw-hs-secret",
              t("settings.hsAppSecret"),
              <Input
                id="tw-hs-secret"
                type="password"
                autoComplete="off"
                placeholder={settings.helpscout.has_app_secret ? "••••••••" : ""}
                onChange={(e) => set({ helpscout_app_secret: e.target.value })}
              />,
            )}
            {field(
              "tw-hs-hook",
              t("settings.hsWebhookSecret"),
              <Input
                id="tw-hs-hook"
                type="password"
                autoComplete="off"
                placeholder={settings.helpscout.has_webhook_secret ? "••••••••" : ""}
                onChange={(e) => set({ helpscout_webhook_secret: e.target.value })}
              />,
              t("settings.hsHint"),
            )}
          </fieldset>

          {field(
            "tw-forward",
            t("settings.forward"),
            <Input id="tw-forward" type="email" defaultValue={settings.forward_to ?? ""} onChange={(e) => set({ forward_to: e.target.value })} />,
            t("settings.forwardHint"),
          )}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              {tc("cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending && <Loader2 className="animate-spin" />} {tc("save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
