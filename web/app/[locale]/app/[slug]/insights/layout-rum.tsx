"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink, Loader2, Ruler, Wrench } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { toast } from "sonner";
import { CodeBlock } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import { api, qk, type LayoutFix, type LayoutRow } from "@/lib/api";
import { Section, usePct } from "./parts";

const snippet = (key: string) => `import { watchLayout } from "@glot-i18n/insights/layout";

watchLayout({ key: "${key}" }); // 1 % of page views, [data-glot-key] elements`;

const band = (r: LayoutRow) => (r.vw_max ? `${r.vw_min}–${r.vw_max} px` : `≥ ${r.vw_min} px`);

/** Layout RUM: strings that overflow for real visitors, worst first, and the repairs glot proposed. */
export function LayoutRum({ slug, publicKey }: { slug: string; publicKey: string }) {
  const t = useTranslations("insights.layout");
  const format = useFormatter();
  const pct = usePct();
  const qc = useQueryClient();
  const key = [...qk.insights(slug), "layout"];
  const q = useQuery({ queryKey: key, queryFn: () => api.layoutRum(slug) });
  const repair = useMutation({
    mutationFn: () => api.repairLayout(slug),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: key });
      qc.invalidateQueries({ queryKey: qk.feedback(slug) });
      toast.success(t("repaired", { n: r.created }));
    },
    onError: (e) => toast.error(e.message),
  });
  const d = q.data;
  const rows = d?.rows.slice(0, 12) ?? [];

  return (
    <Section
      title={t("title")}
      subtitle={t("subtitle")}
      actions={
        d && d.rows.length > 0 ? (
          <Button size="sm" variant="outline" onClick={() => repair.mutate()} disabled={repair.isPending}>
            {repair.isPending ? <Loader2 className="animate-spin" /> : <Wrench />} {t("repair")}
          </Button>
        ) : undefined
      }
    >
      {!d ? (
        <div className="h-24 animate-pulse rounded-lg bg-muted/40" aria-busy="true" />
      ) : d.rows.length === 0 ? (
        <div className="space-y-3">
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <Ruler className="size-4 shrink-0" /> {t("empty")}
          </p>
          <CodeBlock code={snippet(publicKey)} />
        </div>
      ) : (
        <div className="space-y-5">
          <div className="-mx-4 overflow-x-auto sm:-mx-5">
            <table className="w-full min-w-[640px] text-sm">
              <caption className="sr-only">{t("tableCaption", { min: pct(d.min_rate, 0), views: d.min_views })}</caption>
              <thead className="border-y bg-muted/40 text-xs text-muted-foreground">
                <tr>
                  <th scope="col" className="px-4 py-2 text-start font-medium sm:ps-5">{t("key")}</th>
                  <th scope="col" className="px-4 py-2 text-start font-medium">{t("viewport")}</th>
                  <th scope="col" className="w-[28%] px-4 py-2 text-start font-medium">{t("rate")}</th>
                  <th scope="col" className="px-4 py-2 text-end font-medium sm:pe-5">{t("width")}</th>
                </tr>
              </thead>
              <tbody className="divide-y">
                {rows.map((r) => {
                  const hot = r.rate > d.min_rate && r.views >= d.min_views;
                  return (
                    <tr key={`${r.key}|${r.locale}|${r.vw_min}`}>
                      <td className="max-w-0 px-4 py-2.5 sm:ps-5">
                        <div className="truncate font-mono text-xs" title={r.key}>{r.key}</div>
                        <div className="truncate text-xs text-muted-foreground" title={r.current ?? undefined}>
                          <span className="font-medium uppercase">{r.locale}</span> · {r.current ?? t("unknownKey")}
                        </div>
                      </td>
                      <td className="px-4 py-2.5 whitespace-nowrap tabular-nums">{band(r)}</td>
                      <td className="px-4 py-2.5">
                        <div className="flex items-center gap-2">
                          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted" aria-hidden="true">
                            <div className={hot ? "h-full bg-destructive" : "h-full bg-warning"} style={{ width: `${Math.min(100, r.rate * 100)}%` }} />
                          </div>
                          <span className="w-12 text-end tabular-nums">{pct(r.rate, 0)}</span>
                        </div>
                        <div className="mt-0.5 text-xs text-muted-foreground">
                          {t("views", { n: r.views })}
                          {r.fallback > 0 && ` · ${t("fallback", { n: r.fallback })}`}
                        </div>
                      </td>
                      <td className="px-4 py-2.5 text-end whitespace-nowrap tabular-nums sm:pe-5">
                        {r.overflows > r.fallback ? t("px", { need: r.need_px, box: r.box_px }) : "—"}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>

          {d.fixes.length > 0 && (
            <div>
              <h3 className="mb-2 text-sm font-medium">{t("fixes")}</h3>
              <ul className="space-y-2">
                {d.fixes.map((f) => (
                  <FixItem key={f.id} f={f} slug={slug} when={format.relativeTime(new Date(f.created_at))} />
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </Section>
  );
}

function FixItem({ f, slug, when }: { f: LayoutFix; slug: string; when: string }) {
  const t = useTranslations("insights.layout");
  return (
    <li className="rounded-lg border p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs">{f.key}</span>
        <Badge variant={f.status === "failed" ? "destructive" : f.status === "pr" ? "default" : "secondary"}>
          {t(`status.${f.status}`)}
        </Badge>
        <span className="ms-auto text-xs text-muted-foreground">{when}</span>
      </div>
      <p className="mt-1 text-muted-foreground">{f.evidence}</p>
      {f.proposal && (
        <p className="mt-1 break-words">
          <del className="text-muted-foreground">{f.previous}</del> → <ins className="font-medium no-underline">{f.proposal}</ins>
        </p>
      )}
      {f.error && <p className="mt-1 text-xs text-destructive">{f.error}</p>}
      {f.pr_url ? (
        <a href={f.pr_url} target="_blank" rel="noreferrer" className="mt-1 inline-flex items-center gap-1 text-xs underline-offset-4 hover:underline">
          {t("openPr")} <ExternalLink className="size-3" aria-hidden="true" />
        </a>
      ) : (
        f.status === "suggested" && (
          <Link href={`/app/${slug}/feedback`} className="mt-1 inline-block text-xs underline-offset-4 hover:underline">
            {t("review")}
          </Link>
        )
      )}
    </li>
  );
}
