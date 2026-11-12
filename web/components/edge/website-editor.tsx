"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { ErrorState, useLocaleName } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import type { Project } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";
import { type EdgeSettings, edgeApi, edgeKeys, type WebsitePage } from "./api";

type Seg = { hash: string; context: string; source: string; value: string | null; attr?: string };
type Rendered = { srcdoc: string; segments: Seg[]; selectors: (i: number) => { selector: string; count: number }[]; translated: number };
type Engine = { render: (o: { html: string; base: string; path: string; source: string; dir?: string; tm: WebsitePage["tm"]; skip?: string[] }) => Rendered };
type Pick = { seg: Seg; index: number; x: number; y: number };

/** /widget/glot-editor.js (packages/edge/src/editor.global.ts): the tag's own segmenter and writer, loaded once. */
let engine: Promise<Engine> | undefined;
const loadEngine = () =>
  (engine ??= new Promise<Engine>((ok, fail) => {
    const s = document.createElement("script");
    s.src = "/widget/glot-editor.js";
    s.onload = () => ok((window as unknown as { glotEditor: Engine }).glotEditor);
    s.onerror = () => fail(new Error("glot-editor.js"));
    document.head.append(s);
  }));
const RTL = /^(ar|he|fa|ur|ps|sd|yi|dv|ckb)\b/i;

/**
 * The customer's page, translated as the tag will translate it, before anything is installed: a script-less
 * sandboxed frame (opaque origin). Click a text → correct its translation, or exclude its block for good.
 * `refresh` = a translation job is running: the page is re-read every few seconds until text arrives.
 */
export function WebsiteEditor({ project, settings, site, refresh }: { project: Project; settings: EdgeSettings; site: string; refresh: boolean }) {
  const t = useTranslations("edge.website.editor");
  const slug = project.slug;
  const qc = useQueryClient();
  const message = useErrorMessage();
  const localeName = useLocaleName();
  const locales = settings.website_snippet.locales.length ? settings.website_snippet.locales : project.target_locales;
  const [locale, setLocale] = useState(locales[0] ?? "");
  const [path, setPath] = useState("/");
  const [typed, setTyped] = useState("/");
  const [pick, setPick] = useState<Pick | null>(null);
  const [value, setValue] = useState("");
  const [excluding, setExcluding] = useState(false);
  const frame = useRef<HTMLIFrameElement>(null);
  const scroll = useRef(0);
  const complete = useRef(false);

  const key = [...edgeKeys.all(slug), "website-page", site, path, locale] as const;
  const page = useQuery({
    queryKey: key,
    queryFn: async () => ({ page: await edgeApi.websitePage(slug, path, locale), engine: await loadEngine() }),
    enabled: !!locale && !!site,
    // Translations of a fresh crawl land within seconds: look again while some are missing and nothing is open —
    // for a minute at most (each look reads the customer's page once more).
    refetchInterval: (q) => (refresh && !pick && !complete.current && q.state.dataUpdateCount < 20 ? 3000 : false),
  });
  const out = useMemo(() => {
    if (!page.data) return null;
    const { page: p, engine: e } = page.data;
    return e.render({ html: p.html, base: p.url, path: p.path, source: p.source, dir: RTL.test(locale) ? "rtl" : "ltr", tm: p.tm, skip: p.skip });
  }, [page.data, locale]);

  useEffect(() => {
    complete.current = !!out && out.translated >= out.segments.length;
  }, [out]);

  // Clicks relayed by the frame (it cannot reach this page: opaque origin, only our relay script runs in it).
  useEffect(() => {
    const on = (e: MessageEvent) => {
      const f = frame.current;
      if (!f || e.source !== f.contentWindow || e.data?.glot !== "pick" || !out) return;
      const index = Number(String(e.data.segs).split(" ")[0]);
      const seg = out.segments[index];
      if (!seg) return;
      scroll.current = Number(e.data.scroll) || 0;
      const r = f.getBoundingClientRect();
      const y = Math.min(Math.max(Number(e.data.rect?.y) + Number(e.data.rect?.h) + 8, 8), r.height - 240);
      const x = Math.min(Math.max(Number(e.data.rect?.x), 8), Math.max(8, r.width - 400));
      setPick({ seg, index, x, y });
      setValue(seg.value ?? seg.source);
      setExcluding(false);
    };
    window.addEventListener("message", on);
    return () => window.removeEventListener("message", on);
  }, [out]);

  const save = useMutation({
    mutationFn: () => edgeApi.putSegment(slug, pick!.seg.hash, locale, { context: pick!.seg.context, value }),
    onSuccess: () => {
      toast.success(t("saved"));
      setPick(null);
      void qc.invalidateQueries({ queryKey: key });
    },
    onError: (e) => toast.error(message(e)),
  });
  const exclude = useMutation({
    mutationFn: (selector: string) => {
      const cur = qc.getQueryData<EdgeSettings>(edgeKeys.settings(slug)) ?? settings;
      return edgeApi.putSettings(slug, { ...cur, selectors_exclude: [...new Set([...cur.selectors_exclude, selector])] });
    },
    onSuccess: (r) => {
      qc.setQueryData(edgeKeys.settings(slug), r);
      toast.success(t("excluded"));
      setPick(null);
      void qc.invalidateQueries({ queryKey: key });
    },
    onError: (e) => toast.error(message(e)),
  });
  const candidates = pick && excluding && out ? out.selectors(pick.index) : [];

  if (!locale) return null;
  return (
    <div className="space-y-3">
      <form
        className="flex flex-wrap items-end gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          setPick(null);
          setPath(typed.trim() || "/");
        }}
      >
        <div className="space-y-1.5">
          <Label htmlFor="website-editor-locale">{t("locale")}</Label>
          <Select value={locale} onValueChange={(v) => (setPick(null), setLocale(v))}>
            <SelectTrigger id="website-editor-locale" className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {locales.map((l) => (
                <SelectItem key={l} value={l}>
                  {localeName(l)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="website-editor-path">{t("page")}</Label>
          <Input id="website-editor-path" value={typed} onChange={(e) => setTyped(e.target.value)} className="w-64 font-mono" dir="ltr" translate="no" />
        </div>
        <Button type="submit" variant="outline">
          {t("open")}
        </Button>
        {out && (
          <p className="ms-auto text-xs text-muted-foreground" aria-live="polite">
            {t("count", { translated: out.translated, total: out.segments.length })}
          </p>
        )}
      </form>
      <p className="text-xs text-muted-foreground">{out && out.translated === 0 ? t("empty") : t("hint")}</p>
      {page.isError ? (
        <ErrorState error={page.error} onRetry={() => page.refetch()} />
      ) : !out ? (
        <Skeleton className="h-[32rem] rounded-xl" />
      ) : (
        <div className="relative overflow-hidden rounded-xl border bg-white">
          <iframe
            ref={frame}
            title={t("frame", { language: localeName(locale) })}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            srcDoc={out.srcdoc}
            onLoad={() => frame.current?.contentWindow?.postMessage({ glotScroll: scroll.current }, "*")}
            className="h-[32rem] w-full"
          />
          {pick && (
            <form
              className="absolute z-10 w-96 max-w-[calc(100%-1rem)] space-y-3 rounded-lg border bg-popover p-4 text-popover-foreground shadow-lg"
              style={{ left: pick.x, top: pick.y }}
              onSubmit={(e) => {
                e.preventDefault();
                save.mutate();
              }}
              onKeyDown={(e) => e.key === "Escape" && setPick(null)}
            >
              <div className="space-y-1">
                <p className="text-xs font-medium text-muted-foreground">{t("source")}</p>
                <p className="text-sm" dir="auto" translate="no">
                  {pick.seg.source}
                </p>
              </div>
              {excluding ? (
                <div className="space-y-2">
                  <p className="text-xs font-medium text-muted-foreground">{candidates.length ? t("excludeHint") : t("noSelector")}</p>
                  {candidates.map((c) => (
                    <Button key={c.selector} type="button" variant="outline" size="sm" className="h-auto w-full justify-between gap-3 py-2 font-normal" disabled={exclude.isPending} onClick={() => exclude.mutate(c.selector)}>
                      <code dir="ltr" translate="no" className="truncate font-mono text-xs">
                        {c.selector}
                      </code>
                      <span className="shrink-0 text-xs text-muted-foreground">{t("matches", { count: c.count })}</span>
                    </Button>
                  ))}
                  <Button type="button" variant="ghost" size="sm" onClick={() => setExcluding(false)}>
                    {t("cancel")}
                  </Button>
                </div>
              ) : (
                <>
                  <div className="space-y-1.5">
                    <Label htmlFor="website-editor-value" className="text-xs text-muted-foreground">
                      {t("translation")}
                    </Label>
                    <Textarea id="website-editor-value" autoFocus rows={3} value={value} onChange={(e) => setValue(e.target.value)} lang={locale} dir="auto" />
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <Button type="submit" size="sm" disabled={save.isPending || !value.trim()}>
                      {save.isPending && <Loader2 className="animate-spin" />}
                      {t("save")}
                    </Button>
                    <Button type="button" variant="ghost" size="sm" onClick={() => setPick(null)}>
                      {t("cancel")}
                    </Button>
                    <Button type="button" variant="ghost" size="sm" className="ms-auto text-muted-foreground" onClick={() => setExcluding(true)}>
                      {t("exclude")}
                    </Button>
                  </div>
                </>
              )}
            </form>
          )}
        </div>
      )}
    </div>
  );
}
