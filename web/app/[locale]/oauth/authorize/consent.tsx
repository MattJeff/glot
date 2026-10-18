"use client";

// OAuth consent for remote MCP clients (Lovable, Bolt, v0, ChatGPT, Claude…): pick ONE project and the scopes.
// glot-server validated the request before redirecting here; it re-validates on submit.
import { useQuery } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { Logo } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { api, ApiError, qk } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";

type Info = { client_name: string; redirect_host: string | null; scopes: string[] };
const SCOPES = ["read", "translate", "write"] as const;

export function Consent() {
  const t = useTranslations("oauth");
  const errorMessage = useErrorMessage();
  const params = useSearchParams();
  const query = params.toString();
  const [picked, setProject] = useState("");
  const [scopes, setScopes] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const info = useQuery({
    queryKey: ["oauth", query],
    queryFn: async () => {
      const res = await fetch(`/api/oauth/authorize?${query}`, { credentials: "same-origin" });
      const body = await res.json().catch(() => null);
      if (!res.ok) throw new ApiError(res.status, body?.error?.code ?? "http_error", body?.error?.message ?? res.statusText);
      return body as Info;
    },
    retry: false,
  });
  const projects = useQuery({ queryKey: qk.projects, queryFn: api.projects, enabled: info.isSuccess });

  // Not signed in: log in, then come back to this exact request.
  useEffect(() => {
    if (info.error instanceof ApiError && info.error.status === 401) {
      window.location.assign(`/login?next=${encodeURIComponent(`/oauth/authorize?${query}`)}`);
    }
  }, [info.error, query]);
  // Derived, not synced through an effect: the first project until the user picks one.
  const project = picked || projects.data?.[0]?.slug || "";
  const granted = scopes ?? info.data?.scopes ?? [];

  async function submit(approve: boolean) {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/api/oauth/authorize", {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ ...Object.fromEntries(params.entries()), approve, project, scopes: granted }),
      });
      const body = await res.json().catch(() => null);
      if (!res.ok) throw new ApiError(res.status, body?.error?.code ?? "http_error", body?.error?.message ?? res.statusText);
      window.location.assign(body.redirect);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-screen flex-col">
      <header className="flex h-14 items-center px-4">
        <Logo />
      </header>
      <main className="grid flex-1 place-items-center px-4 pb-20">
        <div className="w-full max-w-md">
          {info.isPending || (info.isError && info.error instanceof ApiError && info.error.status === 401) ? (
            <Loader2 className="mx-auto size-5 animate-spin text-muted-foreground" />
          ) : info.isError ? (
            <div role="alert">
              <h1 className="text-xl font-semibold">{t("invalid")}</h1>
              <p className="mt-2 text-sm text-muted-foreground">{errorMessage(info.error)}</p>
            </div>
          ) : (
            <div className="space-y-6">
              <div>
                <h1 className="text-2xl font-semibold tracking-tight">{t("title", { client: info.data.client_name })}</h1>
                <p className="mt-1 text-sm text-muted-foreground">{t("subtitle")}</p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="oauth-project">{t("project")}</Label>
                {projects.data?.length === 0 ? (
                  <p className="text-sm text-muted-foreground">{t("noProjects")}</p>
                ) : (
                  <select
                    id="oauth-project"
                    className="h-9 w-full rounded-md border border-input bg-transparent px-3 text-sm"
                    value={project}
                    onChange={(e) => setProject(e.target.value)}
                  >
                    {projects.data?.map((p) => (
                      <option key={p.slug} value={p.slug}>
                        {p.name} ({p.slug})
                      </option>
                    ))}
                  </select>
                )}
              </div>
              <fieldset className="space-y-3">
                <legend className="mb-2 text-sm font-medium">{t("scopes")}</legend>
                {SCOPES.filter((s) => info.data.scopes.includes(s)).map((s) => (
                  <div key={s} className="flex items-start gap-3">
                    <Checkbox
                      id={`scope-${s}`}
                      checked={granted.includes(s)}
                      onCheckedChange={(c) => setScopes(c ? [...granted, s] : granted.filter((g) => g !== s))}
                    />
                    <Label htmlFor={`scope-${s}`} className="grid gap-0.5 font-normal">
                      <span className="font-medium">{t(`scope.${s}.title`)}</span>
                      <span className="text-muted-foreground">{t(`scope.${s}.body`)}</span>
                    </Label>
                  </div>
                ))}
              </fieldset>
              {info.data.redirect_host && (
                <p className="text-xs text-muted-foreground">{t("redirect", { host: info.data.redirect_host })}</p>
              )}
              {error && (
                <p role="alert" className="text-sm text-destructive">
                  {error}
                </p>
              )}
              <div className="flex justify-end gap-2">
                <Button variant="outline" disabled={busy} onClick={() => submit(false)}>
                  {t("deny")}
                </Button>
                <Button disabled={busy || !project || granted.length === 0} onClick={() => submit(true)}>
                  {busy && <Loader2 className="animate-spin" />}
                  {t("allow")}
                </Button>
              </div>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
