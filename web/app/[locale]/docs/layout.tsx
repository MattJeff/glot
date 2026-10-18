import { Menu } from "lucide-react";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { LocaleSwitcher, Logo, ThemeToggle } from "@/components/common";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { GITHUB_URL } from "@/lib/snippets";
import { NAV, searchIndex } from "./content";
import { DocsSearch, SidebarNav } from "./docs-ui";
import "./docs.css";

export default async function DocsLayout({ children, params }: LayoutProps<"/[locale]/docs">) {
  const { locale } = await params;
  setRequestLocale(locale as Locale);
  const t = await getTranslations("docs");
  const nav = NAV.map((s) => ({
    section: s.section,
    title: t(`sections.${s.section}`),
    pages: s.pages.map((slug) => ({ slug, title: t(`pages.${slug}`) })),
  }));
  const titles = Object.fromEntries(nav.flatMap((s) => s.pages.map((p) => [p.slug, p.title])));

  return (
    <div className="flex min-h-screen flex-col">
      <a href="#main" className="sr-only z-50 rounded-md bg-background px-3 py-2 focus:not-sr-only focus:fixed focus:start-3 focus:top-3 focus:shadow-lg">
        {t("skip")}
      </a>
      <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur-md">
        {/* Reading progress: a CSS scroll timeline, no JS (hidden where unsupported). */}
        <div aria-hidden className="reading-progress" />
        <div className="mx-auto flex h-14 max-w-7xl items-center gap-4 px-4">
          <Link href="/" aria-label={t("home")}>
            <Logo />
          </Link>
          <Link href="/docs" className="hidden rounded-md px-2 py-1 text-sm font-medium transition-colors hover:bg-muted sm:inline">
            {t("title")}
          </Link>
          <div className="ms-auto flex flex-1 items-center justify-end gap-1">
            <DocsSearch index={searchIndex()} titles={titles} />
            <a
              href={GITHUB_URL}
              target="_blank"
              rel="noreferrer"
              className="hidden px-2 text-sm text-muted-foreground hover:text-foreground md:inline"
            >
              {t("github")}
            </a>
            <LocaleSwitcher />
            <ThemeToggle />
          </div>
        </div>
      </header>
      <div className="mx-auto grid w-full max-w-7xl flex-1 gap-8 px-4 lg:grid-cols-[14rem_1fr]">
        <details className="group border-b py-3 lg:hidden">
          <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium">
            <Menu className="size-4" /> {t("menu")}
          </summary>
          <div className="pt-4">
            <SidebarNav nav={nav} />
          </div>
        </details>
        <aside aria-label={t("menu")} className="sticky top-14 hidden h-[calc(100vh-3.5rem)] overflow-y-auto py-8 pe-2 lg:block">
          <SidebarNav nav={nav} />
        </aside>
        <main id="main" className="min-w-0 py-8">
          {children}
        </main>
      </div>
    </div>
  );
}
