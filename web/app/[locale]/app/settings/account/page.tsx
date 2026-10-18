"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { ErrorState, PageHeader } from "@/components/common";
import { useCelebrationsEnabled } from "@/components/gamification/project-progress";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { api, qk } from "@/lib/api";

const section = "rounded-xl border bg-card p-4 shadow-(--shadow-card) sm:p-6";

export default function AccountPage() {
  const t = useTranslations("account");
  const qc = useQueryClient();
  const me = useQuery({ queryKey: qk.me, queryFn: api.me });

  const prefs = useMutation({
    mutationFn: api.updateMe,
    onSuccess: (m) => qc.setQueryData(qk.me, m),
  });
  const email = useMutation({
    mutationFn: api.changeEmail,
    onSuccess: (r, v) => {
      toast.success(r.pending ? t("email.pending", { email: v.email }) : t("email.changed"));
      qc.invalidateQueries({ queryKey: qk.me });
    },
  });
  const password = useMutation({
    mutationFn: api.changePassword,
    onSuccess: () => toast.success(t("password.changed")),
  });

  if (me.isPending) return <Skeleton className="mx-auto h-80 max-w-2xl rounded-xl" />;
  if (me.isError) return <ErrorState error={me.error} onRetry={() => me.refetch()} />;
  const m = me.data;

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <PageHeader title={t("title")} eyebrow={m.email} subtitle={t("subtitle")} />

      {m.email_enabled && (
        <section className={section}>
          <h2 className="text-base font-semibold">{t("notifications.title")}</h2>
          <div className="mt-4 flex items-start justify-between gap-4">
            <div>
              <Label htmlFor="notify-job">{t("notifications.jobDone")}</Label>
              <p className="mt-1 text-sm text-muted-foreground">{t("notifications.jobDoneHint")}</p>
            </div>
            <Switch
              id="notify-job"
              checked={m.notify_job_done}
              disabled={prefs.isPending}
              onCheckedChange={(v) => prefs.mutate({ notify_job_done: v })}
            />
          </div>
          <p className="mt-4 text-xs text-muted-foreground">{t("notifications.accountNote")}</p>
        </section>
      )}

      <PreferencesSection />

      <section className={section}>
        <h2 className="text-base font-semibold">{t("email.title")}</h2>
        <form
          className="mt-4 space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            email.mutate({ email: String(f.get("email")).trim(), password: String(f.get("password")) });
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor="new-email">{t("email.new")}</Label>
            <Input id="new-email" name="email" type="email" autoComplete="email" required />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="email-password">{t("currentPassword")}</Label>
            <Input id="email-password" name="password" type="password" autoComplete="current-password" required />
          </div>
          {m.email_enabled && <p className="text-xs text-muted-foreground">{t("email.hint")}</p>}
          <Button type="submit" disabled={email.isPending}>
            {email.isPending && <Loader2 className="animate-spin" />}
            {t("email.submit")}
          </Button>
        </form>
      </section>

      <section className={section}>
        <h2 className="text-base font-semibold">{t("password.title")}</h2>
        <form
          className="mt-4 space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            const form = e.currentTarget;
            const f = new FormData(form);
            password.mutate(
              { current_password: String(f.get("current")), password: String(f.get("password")) },
              { onSuccess: () => form.reset() },
            );
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor="current">{t("currentPassword")}</Label>
            <Input id="current" name="current" type="password" autoComplete="current-password" required />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="password">{t("password.new")}</Label>
            <Input id="password" name="password" type="password" autoComplete="new-password" minLength={8} required />
          </div>
          <p className="text-xs text-muted-foreground">{t("password.hint")}</p>
          <Button type="submit" disabled={password.isPending}>
            {password.isPending && <Loader2 className="animate-spin" />}
            {t("password.submit")}
          </Button>
        </form>
      </section>
    </div>
  );
}

function PreferencesSection() {
  const t = useTranslations("account.preferences");
  const [celebrate, setCelebrate] = useCelebrationsEnabled();
  return (
    <section className={section}>
      <h2 className="text-base font-semibold">{t("title")}</h2>
      <div className="mt-4 flex items-start justify-between gap-4">
        <div>
          <Label htmlFor="pref-celebrate">{t("celebrations")}</Label>
          <p id="pref-celebrate-hint" className="mt-1 text-sm text-muted-foreground">{t("celebrationsHint")}</p>
        </div>
        <Switch id="pref-celebrate" aria-describedby="pref-celebrate-hint" checked={celebrate} onCheckedChange={setCelebrate} />
      </div>
    </section>
  );
}
