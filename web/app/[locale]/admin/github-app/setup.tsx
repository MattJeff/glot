"use client";

// Admin: create the deployment's GitHub App in 2 clicks (manifest flow). glot-server stores the credentials
// (private key, webhook secret, client secret encrypted with GLOT_SECRET_KEY) when GitHub redirects back.
import { useMutation, useQuery } from "@tanstack/react-query";
import { CircleCheck, ExternalLink, Loader2, ShieldAlert } from "lucide-react";
import { useTranslations } from "next-intl";
import { useSearchParams } from "next/navigation";
import { useEffect, useId, useRef, useState } from "react";
import { CodeBlock, ErrorState, Logo, ThemeToggle } from "@/components/common";
import { ghKey, githubApi } from "@/components/github/api";
import { GitHubMark } from "@/components/github/connect-github-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Link, useRouter } from "@/i18n/navigation";
import { ApiError } from "@/lib/api";
import { useErrorMessage } from "@/lib/use-error-message";

const PERMISSIONS = ["contents: write", "pull_requests: write", "metadata: read"];
const EVENTS = ["push", "pull_request", "installation"];

export function GitHubAppSetup() {
  const t = useTranslations("githubAdmin");
  const message = useErrorMessage();
  const params = useSearchParams();
  const ids = { name: useId(), nameHelp: useId(), org: useId(), orgHelp: useId() };
  const router = useRouter();
  const app = useQuery({ queryKey: ghKey.app, queryFn: githubApi.app, retry: false });
  const [name, setName] = useState("glot");
  const [org, setOrg] = useState("");
  const form = useRef<HTMLFormElement>(null);
  const [post, setPost] = useState<{ action: string; manifest: string } | null>(null);
  const create = useMutation({ mutationFn: () => githubApi.manifest({ name, org: org || undefined }), onSuccess: setPost });
  // The manifest must be POSTed by the browser to github.com (a form, not fetch).
  useEffect(() => {
    if (post) form.current?.submit();
  }, [post]);
  useEffect(() => {
    if (app.error instanceof ApiError && app.error.status === 401) {
      router.replace(`/login?next=${encodeURIComponent("/admin/github-app")}`);
    }
  }, [app.error, router]);

  let body: React.ReactNode;
  if (app.isPending) body = <Skeleton className="h-48 rounded-xl" />;
  else if (app.isError) body = <ErrorState error={app.error} onRetry={() => app.refetch()} />;
  else if (app.data.configured)
    body = (
      <div className="space-y-4 rounded-xl border bg-card p-5 motion-safe:animate-rise">
        <p role="status" className="flex items-center gap-2 font-medium">
          <CircleCheck className="size-5 text-success-foreground motion-safe:animate-pop" aria-hidden />
          {params.get("created") ? t("created") : t("configured")}
        </p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
          <dt className="text-muted-foreground">{t("slug")}</dt>
          <dd dir="ltr" className="font-mono text-[13px]">{app.data.slug}</dd>
          <dt className="text-muted-foreground">{t("appId")}</dt>
          <dd dir="ltr" className="font-mono text-[13px] tabular-nums">{app.data.app_id}</dd>
        </dl>
        <CodeBlock label={t("webhook")} code={app.data.webhook_url} />
        <div className="flex flex-wrap gap-2">
          <Button asChild variant="outline">
            <a href={app.data.html_url ?? "#"} target="_blank" rel="noreferrer">
              <GitHubMark className="size-4" /> {t("open")} <ExternalLink className="rtl:-scale-x-100" aria-hidden />
            </a>
          </Button>
          <Button asChild variant="ghost">
            <Link href="/app">{t("back")}</Link>
          </Button>
        </div>
        {app.data.can_admin && (
          <details className="text-sm text-muted-foreground">
            <summary className="cursor-pointer rounded-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none">{t("recreate")}</summary>
            <div className="mt-3">{createForm()}</div>
          </details>
        )}
      </div>
    );
  else if (!app.data.can_admin)
    body = (
      <div role="alert" className="flex gap-3 rounded-xl border p-5 text-sm">
        <ShieldAlert className="size-5 shrink-0 text-warning-foreground" aria-hidden />
        <div>
          <p className="font-medium">{t("notAdmin.title")}</p>
          <p className="mt-1 text-muted-foreground">{t("notAdmin.body")}</p>
          <code dir="ltr" className="mt-2 inline-block rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
            GLOT_ADMIN_EMAILS=you@example.com
          </code>
        </div>
      </div>
    );
  else body = <div className="rounded-xl border bg-card p-5">{createForm()}</div>;

  function createForm() {
    return (
      <form
        className="space-y-5"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        {params.get("error") && (
          <p role="alert" className="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm">
            {t("callbackError")}
          </p>
        )}
        <div className="space-y-1.5">
          <Label htmlFor={ids.name}>{t("name")}</Label>
          <Input id={ids.name} value={name} maxLength={34} onChange={(e) => setName(e.target.value)} aria-describedby={ids.nameHelp} />
          <p id={ids.nameHelp} className="text-xs text-muted-foreground">{t("nameHelp")}</p>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor={ids.org}>{t("org")}</Label>
          <Input id={ids.org} dir="ltr" value={org} onChange={(e) => setOrg(e.target.value)} aria-describedby={ids.orgHelp} />
          <p id={ids.orgHelp} className="text-xs text-muted-foreground">{t("orgHelp")}</p>
        </div>
        <div className="space-y-2 text-sm">
          <p className="font-medium">{t("permissions")}</p>
          <ul className="flex flex-wrap gap-1.5" dir="ltr">
            {[...PERMISSIONS, ...EVENTS.map((e) => `webhook: ${e}`)].map((p) => (
              <li key={p} className="rounded-full border px-2.5 py-0.5 font-mono text-xs text-muted-foreground">{p}</li>
            ))}
          </ul>
        </div>
        {create.isError && <p role="alert" className="text-sm text-destructive">{message(create.error)}</p>}
        <Button type="submit" disabled={create.isPending || !!post} aria-busy={create.isPending || !!post}>
          {create.isPending || post ? <Loader2 className="animate-spin" /> : <GitHubMark className="size-4" />}
          {create.isPending || post ? t("redirecting") : t("cta")}
        </Button>
      </form>
    );
  }

  return (
    <div className="min-h-dvh bg-background">
      <header className="mx-auto flex max-w-2xl items-center justify-between px-4 py-6">
        <Link href="/app" aria-label="glot">
          <Logo />
        </Link>
        <ThemeToggle />
      </header>
      <main id="main" className="mx-auto max-w-2xl space-y-6 px-4 pb-16">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">{t("title")}</h1>
          <p className="mt-1.5 text-sm text-pretty text-muted-foreground">{t("subtitle")}</p>
        </div>
        {body}
        {post && (
          <form ref={form} method="post" action={post.action} hidden>
            <input type="hidden" name="manifest" value={post.manifest} />
          </form>
        )}
      </main>
    </div>
  );
}
