"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { CheckCircle2, Loader2, MailWarning, XCircle } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { AuthShell } from "@/components/auth-form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Link } from "@/i18n/navigation";
import { api, qk } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";

const linkCls = "font-medium text-foreground underline-offset-4 hover:underline";

/** /verify-email?token=… — consumes the single-use token once (StrictMode-safe). */
export function VerifyEmail() {
  const t = useTranslations("emailAuth");
  const errorMessage = useErrorMessage();
  const token = useSearchParams().get("token") ?? "";
  const verify = useMutation({ mutationFn: api.verifyEmail, meta: { silent: true } });
  const started = useRef(false);
  useEffect(() => {
    if (token && !started.current) {
      started.current = true;
      verify.mutate(token);
    }
  }, [token, verify]);

  const failed = !token || verify.isError;
  return (
    <AuthShell title={t("verify.title")}>
      <div role="status" className="mt-6 flex items-start gap-3 text-sm">
        {verify.isSuccess ? (
          <>
            <CheckCircle2 className="size-5 shrink-0 text-emerald-600" />
            <p>{t("verify.done")}</p>
          </>
        ) : failed ? (
          <>
            <XCircle className="size-5 shrink-0 text-destructive" />
            <p>{verify.error ? errorMessage(verify.error) : t("invalidLink")}</p>
          </>
        ) : (
          <>
            <Loader2 className="size-5 shrink-0 animate-spin" />
            <p>{t("verify.checking")}</p>
          </>
        )}
      </div>
      <p className="mt-8 text-sm">
        <Link href="/app" className={linkCls}>
          {t("toDashboard")}
        </Link>
      </p>
    </AuthShell>
  );
}

export function ForgotPassword() {
  const t = useTranslations("emailAuth");
  const errorMessage = useErrorMessage();
  const send = useMutation({ mutationFn: api.forgotPassword, meta: { silent: true } });
  return (
    <AuthShell title={t("forgot.title")} subtitle={t("forgot.subtitle")}>
      {send.isSuccess ? (
        <p role="status" className="mt-8 text-sm">
          {t("forgot.sent")}
        </p>
      ) : (
        <form
          className="mt-8 space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            send.mutate(String(new FormData(e.currentTarget).get("email")));
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor="email">{t("email")}</Label>
            <Input id="email" name="email" type="email" autoComplete="email" required />
          </div>
          {send.isError && (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(send.error)}
            </p>
          )}
          <Button type="submit" className="h-9 w-full" disabled={send.isPending}>
            {send.isPending && <Loader2 className="animate-spin" />}
            {t("forgot.submit")}
          </Button>
        </form>
      )}
      <p className="mt-6 text-center text-sm">
        <Link href="/login" className={linkCls}>
          {t("toLogin")}
        </Link>
      </p>
    </AuthShell>
  );
}

/** /reset-password?token=… — new password; success signs out everywhere, so go to login. */
export function ResetPassword() {
  const t = useTranslations("emailAuth");
  const errorMessage = useErrorMessage();
  const token = useSearchParams().get("token") ?? "";
  const [error, setError] = useState<string | null>(null);
  const reset = useMutation({
    mutationFn: api.resetPassword,
    onError: (e) => setError(errorMessage(e)),
    meta: { silent: true },
  });
  if (!token) {
    return (
      <AuthShell title={t("reset.title")}>
        <p role="alert" className="mt-6 text-sm text-destructive">{t("invalidLink")}</p>
        <p className="mt-6 text-sm">
          <Link href="/forgot-password" className={linkCls}>
            {t("reset.again")}
          </Link>
        </p>
      </AuthShell>
    );
  }
  return (
    <AuthShell title={t("reset.title")} subtitle={reset.isSuccess ? undefined : t("reset.subtitle")}>
      {reset.isSuccess ? (
        <p role="status" className="mt-8 text-sm">
          {t("reset.done")}{" "}
          <Link href="/login" className={linkCls}>
            {t("toLogin")}
          </Link>
        </p>
      ) : (
        <form
          className="mt-8 space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            const password = String(f.get("password"));
            if (password.length < 8) return setError(t("passwordTooShort"));
            if (password !== String(f.get("confirm"))) return setError(t("passwordMismatch"));
            setError(null);
            reset.mutate({ token, password });
          }}
        >
          <div className="space-y-1.5">
            <Label htmlFor="password">{t("newPassword")}</Label>
            <Input id="password" name="password" type="password" autoComplete="new-password" minLength={8} required />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="confirm">{t("confirmPassword")}</Label>
            <Input id="confirm" name="confirm" type="password" autoComplete="new-password" minLength={8} required />
          </div>
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <Button type="submit" className="h-9 w-full" disabled={reset.isPending}>
            {reset.isPending && <Loader2 className="animate-spin" />}
            {t("reset.submit")}
          </Button>
        </form>
      )}
    </AuthShell>
  );
}

/** Dashboard banner until the address is confirmed (only when the server can send email). */
export function VerifyEmailBanner() {
  const t = useTranslations("emailAuth");
  const me = useQuery({ queryKey: qk.me, queryFn: api.me });
  const resend = useMutation({
    mutationFn: api.resendVerification,
    onSuccess: () => toast.success(t("banner.sent")),
  });
  if (!me.data || !("email_enabled" in me.data) || !me.data.email_enabled || me.data.email_verified) return null;
  return (
    <div role="note" className="mb-6 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-md border px-3 py-2 text-sm">
      <MailWarning className="size-4 shrink-0 text-muted-foreground" />
      <span className="flex-1">{t("banner.text", { email: me.data.email })}</span>
      <Button size="sm" variant="outline" disabled={resend.isPending || resend.isSuccess} onClick={() => resend.mutate()}>
        {resend.isPending && <Loader2 className="animate-spin" />}
        {t("banner.resend")}
      </Button>
    </div>
  );
}
