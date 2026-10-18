"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { LocaleSwitcher, Logo, ThemeToggle } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Link } from "@/i18n/navigation";
import { api, ApiError, qk } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";

export function AuthForm({ mode }: { mode: "login" | "signup" }) {
  const t = useTranslations("auth");
  const errorMessage = useErrorMessage();
  const qc = useQueryClient();
  const params = useSearchParams();
  const [error, setError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (f: { name: string; email: string; password: string }) =>
      mode === "login" ? api.login(f) : api.signup(f),
    onSuccess: (user) => {
      qc.setQueryData(qk.me, user);
      const next = params.get("next");
      // Full navigation so the proxy sees the fresh session cookie. Only same-site paths are allowed.
      window.location.assign(next?.startsWith("/") && !next.startsWith("//") ? next : "/app");
    },
    onError: (err) =>
      setError(err instanceof ApiError && err.status === 401 ? t("errors.invalid") : errorMessage(err)),
    meta: { silent: true },
  });

  function onSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const f = new FormData(e.currentTarget);
    const password = String(f.get("password"));
    if (mode === "signup" && password.length < 8) return setError(t("errors.passwordTooShort"));
    setError(null);
    mutation.mutate({ name: String(f.get("name") ?? ""), email: String(f.get("email")), password });
  }

  return (
    <div className="flex min-h-screen flex-col">
      <header className="flex h-14 items-center justify-between px-4">
        <Link href="/">
          <Logo />
        </Link>
        <div className="flex gap-1">
          <LocaleSwitcher />
          <ThemeToggle />
        </div>
      </header>
      <main className="grid flex-1 place-items-center px-4 pb-20">
        <div className="w-full max-w-sm">
          <h1 className="text-2xl font-semibold tracking-tight">{t(`${mode}.title`)}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t(`${mode}.subtitle`)}</p>
          <form onSubmit={onSubmit} className="mt-8 space-y-4" noValidate={false}>
            {mode === "signup" && (
              <div className="space-y-1.5">
                <Label htmlFor="name">{t("fields.name")}</Label>
                <Input id="name" name="name" autoComplete="name" required />
              </div>
            )}
            <div className="space-y-1.5">
              <Label htmlFor="email">{t("fields.email")}</Label>
              <Input id="email" name="email" type="email" autoComplete="email" required />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="password">{t("fields.password")}</Label>
              <Input
                id="password"
                name="password"
                type="password"
                autoComplete={mode === "login" ? "current-password" : "new-password"}
                minLength={mode === "signup" ? 8 : undefined}
                aria-describedby={mode === "signup" ? "password-hint" : undefined}
                aria-invalid={!!error}
                required
              />
              {mode === "signup" && (
                <p id="password-hint" className="text-xs text-muted-foreground">
                  {t("fields.passwordHint")}
                </p>
              )}
            </div>
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            <Button type="submit" className="h-9 w-full" disabled={mutation.isPending}>
              {mutation.isPending && <Loader2 className="animate-spin" />}
              {t(`${mode}.submit`)}
            </Button>
          </form>
          <p className="mt-6 text-center text-sm text-muted-foreground">
            {t(`${mode}.switch`)}{" "}
            <Link
              href={mode === "login" ? "/signup" : "/login"}
              className="font-medium text-foreground underline-offset-4 hover:underline"
            >
              {t(`${mode}.switchLink`)}
            </Link>
          </p>
        </div>
      </main>
    </div>
  );
}
