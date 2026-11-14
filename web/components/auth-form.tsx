"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useLocale, useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { LocaleSwitcher, Logo, ThemeToggle } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Link } from "@/i18n/navigation";
import { api, ApiError, qk } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";

// Greetings around the mascot: the product's sample copy, decorative (aria-hidden), not UI text.
const HELLO = [["en", "Hello!", "top-[14%] start-[8%]"], ["fr", "Bonjour !", "top-[10%] end-[10%]"], ["ja", "こんにちは！", "bottom-[26%] end-[6%]"], ["es", "¡Hola!", "bottom-[20%] start-[10%]"]] as const;

/** Login, signup and the email-link pages: the form on a glass card, the landing's mascot scene beside it. */
export function AuthShell({ title, subtitle, children }: { title: string; subtitle?: string; children: React.ReactNode }) {
  return (
    <div className="app-canvas flex min-h-screen flex-col">
      <header className="flex h-16 items-center justify-between px-4 sm:px-6">
        <Link href="/">
          <Logo className="text-xl" />
        </Link>
        <div className="flex gap-1">
          <LocaleSwitcher />
          <ThemeToggle />
        </div>
      </header>
      <main className="mx-auto grid w-full max-w-6xl flex-1 items-center gap-10 px-4 pb-16 lg:grid-cols-2">
        <div className="mx-auto w-full max-w-sm rounded-3xl border bg-card/80 p-7 shadow-[0_30px_80px_-40px_rgb(30_20_60/0.45)] backdrop-blur-xl">
          <h1 className="text-2xl font-bold tracking-tight">{title}</h1>
          {subtitle && <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>}
          {children}
        </div>
        <div aria-hidden className="relative hidden aspect-square overflow-hidden rounded-[2.5rem] border bg-[radial-gradient(60%_50%_at_80%_10%,rgb(85_230_255/0.25),transparent_70%),radial-gradient(50%_45%_at_10%_30%,rgb(255_162_87/0.25),transparent_70%),radial-gradient(55%_50%_at_60%_100%,rgb(181_140_255/0.3),transparent_70%)] lg:block">
          {/* eslint-disable-next-line @next/next/no-img-element -- pre-sized transparent WebP generated with Higgsfield */}
          <img src="/landing/robot-wave.webp" alt="" className="absolute inset-x-0 bottom-0 mx-auto w-[62%] drop-shadow-[0_30px_40px_rgb(0_0_0/0.3)]" />
          {HELLO.map(([code, hi, pos]) => (
            <span key={code} className={`absolute ${pos} flex items-center gap-2 rounded-full border bg-card/85 px-3 py-1.5 text-sm font-semibold shadow-lg backdrop-blur`}>
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img src={`/landing/flags/${code}.webp`} alt="" className="size-5 rounded-full" />
              {hi}
            </span>
          ))}
        </div>
      </main>
    </div>
  );
}

export function AuthForm({ mode }: { mode: "login" | "signup" }) {
  const t = useTranslations("auth");
  const locale = useLocale();
  const errorMessage = useErrorMessage();
  const qc = useQueryClient();
  const params = useSearchParams();
  const [error, setError] = useState<string | null>(null);

  const mutation = useMutation({
    mutationFn: (f: { name: string; email: string; password: string; website: string }) =>
      mode === "login" ? api.login(f) : api.signup({ ...f, locale }),
    onSuccess: async (user) => {
      qc.setQueryData(qk.me, user);
      const next = params.get("next");
      const safeNext = next?.startsWith("/") && !next.startsWith("//") ? next : null;
      let dest = safeNext ?? "/app";
      // Where the server requires a confirmed address, a new account lands on "check your inbox" first.
      if (mode === "signup") {
        const me = await api.me().catch(() => null);
        if (me?.email_enabled && !me.email_verified)
          dest = `/verify-email?sent=1${safeNext ? `&next=${encodeURIComponent(safeNext)}` : ""}`;
      }
      // Full navigation so the proxy sees the fresh session cookie. Only same-site paths are allowed.
      // eslint-disable-next-line @next/next/no-location-assign-relative-destination -- a router push would keep the pre-login cache
      window.location.assign(dest);
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
    mutation.mutate({
      name: String(f.get("name") ?? ""),
      email: String(f.get("email")),
      password,
      website: String(f.get("website") ?? ""),
    });
  }

  return (
    <AuthShell title={t(`${mode}.title`)} subtitle={t(`${mode}.subtitle`)}>
      <form onSubmit={onSubmit} className="mt-8 space-y-4" noValidate={false}>
        {mode === "signup" && (
          // Honeypot: invisible to people and screen readers, filled by form bots (the server refuses those).
          <input type="text" name="website" tabIndex={-1} autoComplete="off" aria-hidden="true" className="sr-only" />
        )}
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
          <div className="flex items-center justify-between">
            <Label htmlFor="password">{t("fields.password")}</Label>
            {mode === "login" && (
              <Link href="/forgot-password" className="text-xs text-muted-foreground underline-offset-4 hover:underline">
                {t("login.forgot")}
              </Link>
            )}
          </div>
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
    </AuthShell>
  );
}
