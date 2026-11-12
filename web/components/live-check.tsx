"use client";

// "Is this language really live on my site?" (crates/server/src/live_check.rs, docs: languages-live-check).
// A language can be 100 % translated in glot while the site's own code ignores the code glot sends
// (`?lang=zh-CN` on a site that only accepts `zh`). Per target: the state, the cause in plain words, the fixes.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleAlert, CircleCheck, CircleHelp, Loader2, RefreshCw } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { CodeBlock, CopyButton, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Link } from "@/i18n/navigation";
import { request } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";
import { useErrorMessage } from "@/lib/use-error-message";

type LiveState = "live" | "not_displayed" | "unknown" | "unchecked" | "no_site";
type LiveLocale = {
  locale: string;
  /** The code the switcher writes for this locale (glot's own unless mapped). */
  site_code: string;
  mapped: boolean;
  auto_applied: boolean;
  state: LiveState;
  evidence: "visitors" | "probe" | null;
  /** A code the probe saw the site honor (never a guess). */
  suggestion: { code: string } | null;
  probe: {
    verdict: "honored" | "ignored" | "unknown" | "unreachable";
    code: string;
    url: string;
    status: number | null;
    html_lang: string | null;
    works_with: string | null;
    error: string | null;
  } | null;
  checked_at: string | null;
  visitors: { applied: number; ignored: number; html_lang: string | null; origin: string | null };
};
type LiveCheck = { site_url: string | null; source: string; site_codes: Record<string, string>; locales: LiveLocale[] };

const base = (slug: string) => `/api/projects/${encodeURIComponent(slug)}`;
const key = (slug: string) => ["live-check", slug] as const;

/** The state of every target; probes the site once when a language has never been checked. */
function useLiveCheck(slug: string) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: key(slug), queryFn: () => request<LiveCheck>("GET", `${base(slug)}/live-check`), staleTime: 30_000 });
  const run = useMutation({
    mutationFn: () => request<LiveCheck>("POST", `${base(slug)}/live-check`),
    onSuccess: (next) => qc.setQueryData(key(slug), next),
  });
  const auto = useRef(false);
  const stale = !!q.data?.site_url && q.data.locales.some((l) => l.state === "unchecked");
  const { mutate } = run;
  useEffect(() => {
    if (!stale || auto.current) return;
    auto.current = true;
    mutate();
  }, [stale, mutate]);
  return { q, run };
}

const code = (c: React.ReactNode) => (
  <code dir="ltr" translate="no" className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
    {c}
  </code>
);

/** Overview: one short line per language, only when there is something to say. */
export function LiveBadge({ slug, locale }: { slug: string; locale: string }) {
  const t = useTranslations("liveCheck");
  const { q } = useLiveCheck(slug);
  const l = q.data?.locales.find((x) => x.locale === locale);
  if (l?.state === "live")
    return (
      <span className="inline-flex items-center gap-1 text-success-foreground">
        <CircleCheck className="size-3.5" aria-hidden /> {t("badge.live")}
      </span>
    );
  if (l?.state === "not_displayed")
    return (
      <span className="inline-flex items-center gap-1 text-warning-foreground">
        <CircleAlert className="size-3.5" aria-hidden /> {t("badge.notDisplayed")}
      </span>
    );
  return null;
}

/** Languages matrix: one line per language the site does not display, with the way to the fix. */
export function LiveCheckAlert({ slug }: { slug: string }) {
  const t = useTranslations("liveCheck");
  const { q } = useLiveCheck(slug);
  const broken = q.data?.locales.filter((l) => l.state === "not_displayed") ?? [];
  if (broken.length === 0) return null;
  return (
    <div role="status" className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-warning/40 bg-warning/10 px-3 py-1.5 text-xs text-warning-foreground">
      <CircleAlert className="size-3.5 shrink-0" aria-hidden />
      <span>
        <span className="font-mono">{broken.map((l) => l.locale).join(", ")}</span> · {t("state.not_displayed")}
      </span>
      <Link href={`/app/${slug}/settings#live-check`} className="font-medium underline underline-offset-2">
        {t("fix.title")}
      </Link>
    </div>
  );
}

/** Shown while a language is added but not saved yet. */
export function LiveCheckNotice() {
  const t = useTranslations("liveCheck");
  return <p className="text-xs text-muted-foreground">{t("willCheck")}</p>;
}

/** Settings → Languages (`editable`: site codes can be changed) and the Languages matrix (problems and states). */
export function LiveCheckPanel({ slug, editable = false }: { slug: string; editable?: boolean }) {
  const t = useTranslations("liveCheck");
  const qc = useQueryClient();
  const errorMessage = useErrorMessage();
  const { q, run } = useLiveCheck(slug);
  const save = useMutation({
    mutationFn: (codes: Record<string, string>) => request<Record<string, string>>("PUT", `${base(slug)}/site-codes`, codes),
    // The server re-checks the changed language in the background: read the state again once it has.
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: key(slug) });
      setTimeout(() => qc.invalidateQueries({ queryKey: key(slug) }), 4000);
    },
    onError: (e) => toast.error(errorMessage(e)),
  });
  if (!q.data || q.data.locales.length === 0) return null;
  const d = q.data;
  const setCode = (locale: string, to: string) => save.mutate({ ...d.site_codes, [locale]: to });
  return (
    // Inside the Settings form: the copy buttons (no `type`) must not submit it.
    <div
      id="live-check"
      className="scroll-mt-20 space-y-3 rounded-lg border p-4"
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button")?.type === "submit") e.preventDefault();
      }}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-medium">{t("title")}</h3>
          <p className="text-sm text-pretty text-muted-foreground">{t("description")}</p>
        </div>
        {d.site_url && (
          <Button type="button" variant="outline" size="sm" disabled={run.isPending} onClick={() => run.mutate()}>
            {run.isPending ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            {run.isPending ? t("checking") : t("recheck")}
          </Button>
        )}
      </div>
      <ul className="divide-y rounded-md border">
        {d.locales.map((l) => (
          <Row key={l.locale} l={l} siteUrl={d.site_url} editable={editable} busy={save.isPending} setCode={setCode} />
        ))}
      </ul>
    </div>
  );
}

function Row({
  l,
  siteUrl,
  editable,
  busy,
  setCode,
}: {
  l: LiveLocale;
  siteUrl: string | null;
  editable: boolean;
  busy: boolean;
  setCode: (locale: string, to: string) => void;
}) {
  const t = useTranslations("liveCheck");
  const tc = useTranslations("common");
  const format = useFormatter();
  const localeName = useLocaleName();
  const origin = useOrigin() ?? "";
  const [draft, setDraft] = useState(l.site_code);
  const [last, setLast] = useState(l.site_code);
  if (last !== l.site_code) {
    setLast(l.site_code);
    setDraft(l.site_code);
  }
  const language = localeName(l.locale);
  const Icon = l.state === "live" ? CircleCheck : l.state === "not_displayed" ? CircleAlert : CircleHelp;
  const tone = l.state === "live" ? "text-success-foreground" : l.state === "not_displayed" ? "text-warning-foreground" : "text-muted-foreground";
  const rich = { c: code };
  const probe = l.probe;
  const docs = `${origin}/docs/languages-live-check`;
  // Generic: wherever the site reads the language, accept a region / script too.
  const snippet = `// Accept "${l.locale}", not only two or three letters
const lang = new URLSearchParams(location.search).get("lang") ?? "";
if (/^[a-z]{2,3}([-_][A-Za-z0-9]{2,8})*$/.test(lang)) setLanguage(lang);`;
  const message = t("fix.message", { language, locale: l.locale, site: l.visitors.origin ?? siteUrl ?? "", docs });
  return (
    <li className="space-y-2 px-3 py-2.5 text-sm">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="font-mono text-xs">{l.locale}</span>
        <span className="text-muted-foreground capitalize">{language}</span>
        <span className={`inline-flex items-center gap-1 font-medium ${tone}`}>
          <Icon className="size-4" aria-hidden /> {t(`state.${l.state}`)}
        </span>
        {l.checked_at && (
          <span className="ms-auto text-xs text-muted-foreground">{t("checked", { date: format.relativeTime(new Date(l.checked_at)) })}</span>
        )}
      </div>

      {l.mapped && (
        <p className="text-pretty text-muted-foreground">
          {t.rich(l.auto_applied ? "mappedAuto" : "mapped", { ...rich, code: l.site_code, locale: l.locale })}
        </p>
      )}
      {l.state === "live" && l.evidence === "visitors" && <p className="text-xs text-muted-foreground">{t("evidence.visitors")}</p>}

      {l.state === "not_displayed" && (
        <div className="space-y-3">
          <p className="text-pretty">
            {l.evidence === "visitors"
              ? t.rich(l.visitors.html_lang ? "cause.visitorsLang" : "cause.visitors", { ...rich, code: l.site_code, lang: l.visitors.html_lang ?? "" })
              : t.rich("cause.probe", { ...rich, code: l.site_code, url: probe?.url ?? "" })}{" "}
            {probe?.works_with && t.rich("cause.worksWith", { ...rich, code: probe.works_with })}
          </p>
          <div className="space-y-3 rounded-md bg-muted/40 p-3">
            <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{t("fix.title")}</p>
            {l.suggestion && (
              <div className="space-y-1">
                <Button type="button" size="sm" disabled={busy} onClick={() => setCode(l.locale, l.suggestion!.code)}>
                  {busy && <Loader2 className="animate-spin" />}
                  {t("fix.alias", { code: l.suggestion.code })}
                </Button>
                <p className="text-xs text-pretty text-muted-foreground">
                  {t.rich("fix.aliasConfirmed", { ...rich, code: l.suggestion.code, locale: l.locale })}
                </p>
              </div>
            )}
            <div className="space-y-1.5">
              <p className="text-pretty">
                {t.rich("fix.dev", {
                  ...rich,
                  locale: l.locale,
                  docs: (c) => (
                    <Link href="/docs/languages-live-check" className="underline underline-offset-2">
                      {c}
                    </Link>
                  ),
                })}
              </p>
              <CodeBlock code={snippet} />
            </div>
            <div className="flex items-start gap-2">
              <div className="min-w-0 flex-1">
                <p className="font-medium">{t("fix.send")}</p>
                <p className="text-xs text-pretty whitespace-pre-line text-muted-foreground">{message}</p>
              </div>
              <CopyButton value={message} />
            </div>
          </div>
        </div>
      )}

      {l.state === "unknown" && (
        <p className="text-pretty text-muted-foreground">
          {probe?.verdict === "unreachable"
            ? t.rich("unknown.unreachable", { ...rich, url: probe.url })
            : probe?.status && probe.status >= 400
              ? t.rich("unknown.http", { ...rich, status: probe.status, url: probe.url })
              : t("unknown.csr")}
        </p>
      )}
      {l.state === "no_site" && <p className="text-pretty text-muted-foreground">{t("noSiteHint")}</p>}

      {editable && (
        <div className="flex flex-wrap items-center gap-2">
          <label htmlFor={`site-code-${l.locale}`} className="text-xs text-muted-foreground">
            {t("codeLabel")}
          </label>
          <Input
            id={`site-code-${l.locale}`}
            value={draft}
            dir="ltr"
            spellCheck={false}
            className="h-7 w-28 font-mono text-xs"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key !== "Enter") return;
              e.preventDefault(); // the page's form must not submit
              if (draft.trim() !== l.site_code) setCode(l.locale, draft.trim() || l.locale);
            }}
          />
          {draft.trim() !== l.site_code && (
            <Button type="button" variant="outline" size="xs" disabled={busy} onClick={() => setCode(l.locale, draft.trim() || l.locale)}>
              {tc("save")}
            </Button>
          )}
          {l.mapped && draft === l.site_code && (
            <Button type="button" variant="ghost" size="xs" disabled={busy} onClick={() => setCode(l.locale, l.locale)}>
              {t("reset", { locale: l.locale })}
            </Button>
          )}
        </div>
      )}
    </li>
  );
}
