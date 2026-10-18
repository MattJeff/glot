"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link2, Loader2, Trash2 } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CopyButton } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { api, ApiError, qk, type ReviewAccess } from "@/lib/api";

const ACCESS: ReviewAccess[] = ["read", "comment", "edit"];
const EXPIRY = ["1", "7", "14", "30", "90", "never"] as const;

/** Review links (contract §2): a reviewer without an account opens `site?glot_review=<token>` and the widget does the rest. */
export function ShareDialog({ slug, siteUrl, open, onOpenChange }: { slug: string; siteUrl: string; open: boolean; onOpenChange: (o: boolean) => void }) {
  const t = useTranslations("reviewPage.share");
  const format = useFormatter();
  const qc = useQueryClient();
  const links = useQuery({ queryKey: qk.reviewLinks(slug), queryFn: () => api.reviewLinks(slug), enabled: open });
  const [label, setLabel] = useState("");
  const [access, setAccess] = useState<ReviewAccess>("comment");
  const [expiry, setExpiry] = useState<(typeof EXPIRY)[number]>("14");
  const [created, setCreated] = useState<string | null>(null);
  const linkUrl = (token: string) => {
    const u = new URL(siteUrl);
    u.searchParams.set("glot_review", token);
    return u.toString();
  };
  const create = useMutation({
    mutationFn: () => api.createReviewLink(slug, { label: label.trim() || undefined, access, expires_in_days: expiry === "never" ? null : Number(expiry) }),
    onSuccess: (r) => {
      setCreated(linkUrl(r.token));
      setLabel("");
      qc.invalidateQueries({ queryKey: qk.reviewLinks(slug) });
    },
  });
  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteReviewLink(slug, id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.reviewLinks(slug) });
      toast.success(t("revoked"));
    },
  });
  const forbidden = links.error instanceof ApiError && links.error.status === 403;
  const active = (links.data ?? []).filter((l) => !l.revoked_at && (!l.expires_at || new Date(l.expires_at) > new Date()));

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        onOpenChange(o);
        if (!o) setCreated(null);
      }}
    >
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("title")}</DialogTitle>
          <DialogDescription>{t("body")}</DialogDescription>
        </DialogHeader>
        {forbidden ? (
          <p className="rounded-md bg-muted p-3 text-sm">{t("adminOnly")}</p>
        ) : (
          <>
            <form
              className="space-y-3"
              onSubmit={(e) => {
                e.preventDefault();
                create.mutate();
              }}
            >
              <div className="space-y-1.5">
                <Label htmlFor="rv-link-label">{t("label")}</Label>
                <Input id="rv-link-label" value={label} onChange={(e) => setLabel(e.target.value)} placeholder={t("labelPlaceholder")} maxLength={80} />
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <Label htmlFor="rv-link-access">{t("access")}</Label>
                  <Select value={access} onValueChange={(v) => setAccess(v as ReviewAccess)}>
                    <SelectTrigger id="rv-link-access" className="w-full" aria-describedby="rv-link-access-hint">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {ACCESS.map((a) => (
                        <SelectItem key={a} value={a}>
                          {t(`accessLevel.${a}`)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p id="rv-link-access-hint" className="text-xs text-muted-foreground">
                    {t(`accessHint.${access}`)}
                  </p>
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="rv-link-expiry">{t("expiry")}</Label>
                  <Select value={expiry} onValueChange={(v) => setExpiry(v as (typeof EXPIRY)[number])}>
                    <SelectTrigger id="rv-link-expiry" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {EXPIRY.map((d) => (
                        <SelectItem key={d} value={d}>
                          {d === "never" ? t("never") : t("days", { count: Number(d) })}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <Button type="submit" disabled={create.isPending}>
                {create.isPending ? <Loader2 className="animate-spin" /> : <Link2 />} {t("create")}
              </Button>
            </form>
            {created && (
              <div role="status" className="space-y-1.5 rounded-lg border border-brand/40 bg-brand/5 p-3">
                <p className="text-sm font-medium">{t("createdTitle")}</p>
                <div className="flex items-center gap-2">
                  <code dir="ltr" className="min-w-0 flex-1 truncate rounded bg-background px-2 py-1 font-mono text-xs">
                    {created}
                  </code>
                  <CopyButton value={created} />
                </div>
                <p className="text-xs text-muted-foreground">{t("createdHint")}</p>
              </div>
            )}
            <section className="space-y-2">
              <h3 className="text-sm font-medium">{t("active", { count: active.length })}</h3>
              {active.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t("none")}</p>
              ) : (
                <ul className="divide-y rounded-lg border">
                  {active.map((l) => (
                    <li key={l.id} className="flex items-center gap-3 px-3 py-2 text-sm">
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-medium">{l.label ?? t("untitled")}</span>
                        <span className="text-xs text-muted-foreground">
                          {t(`accessLevel.${l.access}`)} ·{" "}
                          {l.expires_at ? t("expires", { date: format.dateTime(new Date(l.expires_at), { dateStyle: "medium" }) }) : t("neverExpires")}
                        </span>
                      </span>
                      <Button variant="ghost" size="icon-sm" aria-label={t("revoke", { label: l.label ?? t("untitled") })} disabled={revoke.isPending} onClick={() => revoke.mutate(l.id)}>
                        <Trash2 />
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
