import { ArrowRight, Bot, Smartphone, User, Users } from "lucide-react";
import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { pageUrl } from "@/lib/seo";
import { NAV, type Slug } from "./content";

export async function generateMetadata({ params }: PageProps<"/[locale]/docs">): Promise<Metadata> {
  const { locale } = await params;
  const t = await getTranslations({ locale: locale as Locale, namespace: "docs" });
  const url = pageUrl("en", "/docs");
  return {
    title: t("title"),
    description: t("start.subtitle"),
    alternates: { canonical: url },
    openGraph: { type: "website", url, siteName: "glot", title: `${t("title")} · glot`, description: t("start.subtitle"), locale: "en_US" },
  };
}

// Each path = the pages to read, in order. Anchors exist in web/content/docs/*.md.
const PATHS: { key: "solo" | "team" | "agent" | "mobile"; icon: typeof User; steps: { slug: Slug; hash?: string }[]; cmd: string }[] = [
  { key: "solo", icon: User, cmd: "npx glot-i18n localize", steps: [{ slug: "getting-started" }, { slug: "concepts" }, { slug: "faq" }] },
  { key: "team", icon: Users, cmd: "glot check --format github", steps: [{ slug: "integrations", hash: "github-action" }, { slug: "quality" }, { slug: "self-hosting" }] },
  { key: "agent", icon: Bot, cmd: "npx -y glot-i18n mcp", steps: [{ slug: "mcp" }, { slug: "integrations" }, { slug: "skills" }] },
  { key: "mobile", icon: Smartphone, cmd: "npx glot-i18n init", steps: [{ slug: "getting-started", hash: "1-glot-init" }, { slug: "frameworks" }, { slug: "cli" }] },
];

export default async function DocsHome({ params }: PageProps<"/[locale]/docs">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("docs");

  return (
    <div className="max-w-4xl">
      <h1 className="text-4xl font-semibold tracking-tight text-balance">{t("start.title")}</h1>
      <p className="mt-3 max-w-2xl text-lg text-pretty text-muted-foreground">{t("start.subtitle")}</p>

      <h2 className="mt-12 text-sm font-semibold tracking-wide text-foreground/80 uppercase">{t("start.paths")}</h2>
      <ul className="mt-4 grid gap-4 sm:grid-cols-2">
        {PATHS.map(({ key, icon: Icon, steps, cmd }) => (
          <li key={key} className="flex flex-col rounded-xl border bg-card p-5 shadow-xs">
            <div className="flex items-center gap-2.5">
              <span className="grid size-8 place-items-center rounded-lg border bg-background">
                <Icon aria-hidden className="size-4 text-brand" />
              </span>
              <h3 className="font-semibold">{t(`start.${key}.title`)}</h3>
            </div>
            <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{t(`start.${key}.body`)}</p>
            <code dir="ltr" className="mt-4 block truncate rounded-md border bg-muted/50 px-2.5 py-1.5 font-mono text-xs">
              {cmd}
            </code>
            <ol className="mt-4 space-y-1 text-sm">
              {steps.map((s, i) => (
                <li key={s.slug + (s.hash ?? "")}>
                  <Link
                    href={`/docs/${s.slug}${s.hash ? `#${s.hash}` : ""}`}
                    className="group flex min-h-8 items-center gap-2.5 rounded-md px-1.5 hover:bg-muted"
                  >
                    <span className="grid size-5 shrink-0 place-items-center rounded-full border font-mono text-[10px] tabular-nums text-muted-foreground">{i + 1}</span>
                    <span className="flex-1">{t(`pages.${s.slug}`)}</span>
                    <ArrowRight aria-hidden className="size-3.5 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 rtl:rotate-180" />
                  </Link>
                </li>
              ))}
            </ol>
          </li>
        ))}
      </ul>

      <h2 className="mt-14 text-sm font-semibold tracking-wide text-foreground/80 uppercase">{t("start.all")}</h2>
      <div className="mt-4 grid gap-8 sm:grid-cols-2 lg:grid-cols-3">
        {NAV.map((s) => (
          <section key={s.section} aria-labelledby={`sec-${s.section}`}>
            <h3 id={`sec-${s.section}`} className="font-medium">{t(`sections.${s.section}`)}</h3>
            <ul className="mt-2 space-y-1 text-sm">
              {s.pages.map((p) => (
                <li key={p}>
                  <Link href={`/docs/${p}`} className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
                    {t(`pages.${p}`)}
                  </Link>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}
