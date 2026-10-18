import {
  ArrowRight,
  Bot,
  BookOpenText,
  Check,
  CircleAlert,
  Eye,
  GitPullRequest,
  KeyRound,
  Lock,
  Minus,
  Plug,
  ShieldCheck,
  Star,
  X,
} from "lucide-react";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { CodeBlock, LocaleSwitcher, Logo, ThemeToggle } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { DEMO, GITHUB_URL, INSTALL_CARGO, INSTALL_NPX, mcpJson } from "@/lib/snippets";
import { cn } from "@/lib/utils";

export default async function Landing({ params }: PageProps<"/[locale]">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("landing");
  const tc = await getTranslations("common");

  const features = [
    { icon: ShieldCheck, key: "validator", wide: true },
    { icon: Lock, key: "lockfile" },
    { icon: BookOpenText, key: "glossary" },
    { icon: Plug, key: "mcp" },
    { icon: Eye, key: "visual" },
    { icon: KeyRound, key: "byok" },
    { icon: Star, key: "oss" },
  ] as const;

  type Cell = "yes" | "no" | "partial" | string;
  const rows: { label: string; glot: Cell; lokalise: Cell; crowdin: Cell }[] = [
    { label: t("compare.rows.price"), glot: t("compare.glotPrice"), lokalise: t("compare.lokalisePrice"), crowdin: t("compare.crowdinPrice") },
    { label: t("compare.rows.openSource"), glot: "yes", lokalise: "no", crowdin: "no" },
    { label: t("compare.rows.byok"), glot: "yes", lokalise: "no", crowdin: "partial" },
    { label: t("compare.rows.validator"), glot: "yes", lokalise: "partial", crowdin: "partial" },
    { label: t("compare.rows.gitNative"), glot: "yes", lokalise: "no", crowdin: "no" },
    { label: t("compare.rows.mcp"), glot: "yes", lokalise: "partial", crowdin: "partial" },
  ];
  const cell = (v: Cell) => {
    if (v === "yes") return <Check aria-label={t("compare.yes")} className="mx-auto size-4 text-success" />;
    if (v === "no") return <X aria-label={t("compare.no")} className="mx-auto size-4 text-muted-foreground" />;
    if (v === "partial") return <Minus aria-label={t("compare.partial")} className="mx-auto size-4 text-warning" />;
    return <span className="text-xs">{v}</span>;
  };

  return (
    <div className="flex min-h-screen flex-col">
      <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-6 px-4">
          <Link href="/" aria-label={t("hero.title")}>
            <Logo />
          </Link>
          <nav className="hidden items-center gap-5 text-sm text-muted-foreground md:flex">
            <a href="#features" className="hover:text-foreground">{t("nav.features")}</a>
            <a href="#compare" className="hover:text-foreground">{t("nav.compare")}</a>
            <a href="#install" className="hover:text-foreground">{t("nav.install")}</a>
          </nav>
          <div className="ms-auto flex items-center gap-1">
            <LocaleSwitcher />
            <ThemeToggle />
            <Button variant="ghost" size="sm" asChild className="hidden sm:inline-flex">
              <Link href="/login">{t("nav.signIn")}</Link>
            </Button>
            <Button size="sm" asChild>
              <Link href="/signup">{t("nav.getStarted")}</Link>
            </Button>
          </div>
        </div>
      </header>

      <main id="main" className="flex-1">
        {/* Hero */}
        <section className="relative overflow-hidden border-b">
          <div aria-hidden className="bg-grid absolute inset-0 [mask-image:radial-gradient(ellipse_at_top,black,transparent_70%)] opacity-60" />
          <div className="relative mx-auto max-w-6xl px-4 pt-20 pb-16 text-center sm:pt-28">
            <Badge variant="outline" className="mb-6 gap-1.5 rounded-full bg-background px-3 py-1 text-xs font-normal">
              <span className="size-1.5 rounded-full bg-brand" />
              {t("hero.badge")}
            </Badge>
            <h1 className="mx-auto max-w-3xl text-4xl font-semibold tracking-tight text-balance sm:text-6xl">
              {t("hero.title")}
            </h1>
            <p className="mx-auto mt-5 max-w-2xl text-base text-pretty text-muted-foreground sm:text-lg">
              {t("hero.subtitle")}
            </p>
            <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
              <Button size="lg" asChild className="h-10 px-4">
                <Link href="/signup">
                  {t("hero.primary")} <ArrowRight className="rtl:rotate-180" />
                </Link>
              </Button>
              <Button size="lg" variant="outline" asChild className="h-10 px-4">
                <a href={GITHUB_URL} target="_blank" rel="noreferrer">
                  <Star /> {t("hero.secondary")}
                </a>
              </Button>
            </div>
            <CodeBlock code={INSTALL_CARGO} className="mx-auto mt-6 max-w-xs text-start" />
          </div>

          {/* Flow demo: agent -> MCP -> validation -> PR */}
          <div className="relative mx-auto max-w-6xl px-4 pb-20">
            <h2 className="mb-6 text-center text-sm font-medium text-muted-foreground">{t("flow.title")}</h2>
            <ol className="grid gap-3 md:grid-cols-4">
              <FlowStep n={1} icon={Bot} label={t("flow.agent.label")} caption={t("flow.agent.caption")}>
                <p className="rounded-md bg-muted px-2.5 py-2 text-xs">
                  <span className="text-muted-foreground">&gt; </span>
                  {t("flow.agent.prompt")}
                </p>
                <pre dir="ltr" className="mt-2 overflow-x-auto font-mono text-[11px] leading-relaxed text-muted-foreground">{DEMO.source}</pre>
              </FlowStep>
              <FlowStep n={2} icon={Plug} label={t("flow.mcp.label")} caption={t("flow.mcp.caption")}>
                <pre dir="ltr" className="overflow-x-auto font-mono text-[11px] leading-relaxed">{DEMO.toolCall}</pre>
              </FlowStep>
              <FlowStep n={3} icon={ShieldCheck} label={t("flow.validate.label")} caption={t("flow.validate.caption")}>
                <ul className="space-y-1.5 text-xs">
                  {(["placeholders", "plurals", "glossary"] as const).map((k) => (
                    <li key={k} className="flex items-center gap-2">
                      <Check className="size-3.5 shrink-0 text-success" /> {t(`flow.checks.${k}`)}
                    </li>
                  ))}
                  <li className="flex items-center gap-2">
                    <CircleAlert className="size-3.5 shrink-0 text-warning" /> {t("flow.checks.length")}
                  </li>
                </ul>
              </FlowStep>
              <FlowStep n={4} icon={GitPullRequest} label={t("flow.pr.label")} caption={t("flow.pr.caption")}>
                <div dir="ltr" className="overflow-hidden rounded-md border font-mono text-[11px]">
                  <div className="border-b bg-muted/60 px-2 py-1 text-muted-foreground">{DEMO.file}</div>
                  {DEMO.diff.map((d) => (
                    <div
                      key={d.text}
                      className={cn("truncate px-2 py-0.5", d.sign === "+" ? "bg-success/10 text-success" : "text-muted-foreground")}
                    >
                      {d.sign} {d.text}
                    </div>
                  ))}
                </div>
              </FlowStep>
            </ol>
          </div>
        </section>

        {/* Features */}
        <section id="features" className="mx-auto max-w-6xl scroll-mt-16 px-4 py-20">
          <h2 className="text-3xl font-semibold tracking-tight">{t("features.title")}</h2>
          <p className="mt-2 text-muted-foreground">{t("features.subtitle")}</p>
          <div className="mt-10 grid gap-px overflow-hidden rounded-xl border bg-border sm:grid-cols-2 lg:grid-cols-4">
            {features.map(({ icon: Icon, key, ...f }) => (
              <div key={key} className={cn("bg-background p-6", "wide" in f && f.wide && "lg:col-span-2")}>
                <Icon className="size-5 text-brand" />
                <h3 className="mt-4 font-medium">{t(`features.${key}.title`)}</h3>
                <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{t(`features.${key}.body`)}</p>
              </div>
            ))}
          </div>
        </section>

        {/* Compare */}
        <section id="compare" className="scroll-mt-16 border-y bg-muted/30">
          <div className="mx-auto max-w-6xl px-4 py-20">
            <h2 className="text-3xl font-semibold tracking-tight">{t("compare.title")}</h2>
            <p className="mt-2 text-muted-foreground">{t("compare.subtitle")}</p>
            <div className="mt-10 overflow-x-auto rounded-xl border bg-background">
              <table className="w-full min-w-[560px] text-sm">
                <thead>
                  <tr className="border-b text-muted-foreground">
                    <th scope="col" className="px-4 py-3 text-start font-medium">{t("compare.feature")}</th>
                    <th scope="col" className="bg-brand/5 px-4 py-3 text-center font-semibold text-foreground">
                      <Logo className="justify-center" />
                    </th>
                    {/* Vendor names are proper nouns. */}
                    <th scope="col" className="px-4 py-3 text-center font-medium">{"Lokalise"}</th>
                    <th scope="col" className="px-4 py-3 text-center font-medium">{"Crowdin"}</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((r) => (
                    <tr key={r.label} className="border-b last:border-0">
                      <th scope="row" className="px-4 py-3 text-start font-normal">{r.label}</th>
                      <td className="bg-brand/5 px-4 py-3 text-center font-medium">{cell(r.glot)}</td>
                      <td className="px-4 py-3 text-center text-muted-foreground">{cell(r.lokalise)}</td>
                      <td className="px-4 py-3 text-center text-muted-foreground">{cell(r.crowdin)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <p className="mt-3 text-xs text-muted-foreground">{t("compare.footnote")}</p>
          </div>
        </section>

        {/* Install */}
        <section id="install" className="mx-auto max-w-6xl scroll-mt-16 px-4 py-20">
          <h2 className="text-3xl font-semibold tracking-tight">{t("install.title")}</h2>
          <p className="mt-2 text-muted-foreground">{t("install.subtitle")}</p>
          <div className="mt-10 grid gap-4 lg:grid-cols-2">
            <div className="space-y-4">
              <CodeBlock label={t("install.cargo")} code={INSTALL_CARGO} />
              <CodeBlock label={t("install.npx")} code={INSTALL_NPX} />
            </div>
            <CodeBlock label={t("install.mcp")} code={mcpJson()} />
          </div>
        </section>

        <section className="border-t">
          <div className="mx-auto flex max-w-6xl flex-col items-center gap-5 px-4 py-20 text-center">
            <h2 className="text-3xl font-semibold tracking-tight text-balance">{t("cta.title")}</h2>
            <Button size="lg" asChild className="h-10 px-4">
              <Link href="/signup">
                {t("cta.button")} <ArrowRight className="rtl:rotate-180" />
              </Link>
            </Button>
          </div>
        </section>
      </main>

      <footer className="border-t">
        <div className="mx-auto flex max-w-6xl flex-wrap items-center gap-4 px-4 py-8 text-sm text-muted-foreground">
          <Logo className="text-foreground" />
          <span>{t("footer.tagline")}</span>
          <span className="ms-auto">{t("footer.license")}</span>
          <a href={GITHUB_URL} target="_blank" rel="noreferrer" className="hover:text-foreground">
            {tc("github")}
          </a>
        </div>
      </footer>
    </div>
  );
}

function FlowStep({
  n,
  icon: Icon,
  label,
  caption,
  children,
}: {
  n: number;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  caption: string;
  children: React.ReactNode;
}) {
  return (
    <li className="relative flex flex-col rounded-xl border bg-card p-4 shadow-xs">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid size-6 place-items-center rounded-md border bg-muted font-mono text-[11px]">{n}</span>
        <Icon className="size-4 text-brand" />
        <span className="text-sm font-medium">{label}</span>
      </div>
      <div className="flex-1">{children}</div>
      <p className="mt-3 text-xs text-muted-foreground">{caption}</p>
      {n < 4 && (
        <ArrowRight aria-hidden className="absolute top-1/2 -end-3 z-10 hidden size-4 -translate-y-1/2 rounded-full bg-background text-muted-foreground md:block rtl:rotate-180" />
      )}
    </li>
  );
}
