"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  BookOpenText,
  CreditCard,
  FlaskConical,
  FolderKanban,
  Grid3x3,
  KeyRound,
  UserRound,
  LayoutDashboard,
  ListChecks,
  LogOut,
  Sparkles,
  X,
  Menu,
  MessageSquareWarning,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Radar,
  ScanEye,
  Search,
  Settings,
  TableProperties,
  Languages,
  GitPullRequest,
} from "lucide-react";
import { useLocale, useTranslations } from "next-intl";
import { useTheme } from "next-themes";
import { useEffect, useState, useSyncExternalStore, ViewTransition } from "react";
import { Logo, LocaleSwitcher, ThemeToggle, useLocaleName, useSwitchLocale } from "@/components/common";
import { CreateProjectDialog } from "@/components/create-project-dialog";
import { ProjectCelebrations, SidebarProjectProgress } from "@/components/gamification/project-progress";
import { VerifyEmailBanner } from "@/components/email-pages";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Link, usePathname, useRouter } from "@/i18n/navigation";
import { routing, RTL_LOCALES } from "@/i18n/routing";
import { api, qk } from "@/lib/api";
import { useSlug, useTranslateMissing } from "@/lib/hooks";
import { cn } from "@/lib/utils";

const PROJECT_PAGES = [
  { seg: "", icon: LayoutDashboard, label: "overview" },
  { seg: "/keys", icon: TableProperties, label: "keys" },
  { seg: "/preview", icon: Grid3x3, label: "preview" },
  { seg: "/visual", icon: ScanEye, label: "visual" },
  { seg: "/glossary", icon: BookOpenText, label: "glossary" },
  { seg: "/jobs", icon: ListChecks, label: "jobs" },
  { seg: "/github", icon: GitPullRequest, label: "github" },
  { seg: "/settings", icon: Settings, label: "settings" },
  // Labs (heading before /insights): growth features, less proven than the core loop.
  { seg: "/insights", icon: Radar, label: "insights" },
  { seg: "/experiments", icon: FlaskConical, label: "experiments" },
  { seg: "/feedback", icon: MessageSquareWarning, label: "feedback" },
] as const;

function SidebarNav({
  onNavigate,
  onCreate,
  collapsed = false,
  onToggle,
}: {
  onNavigate?: () => void;
  onCreate: () => void;
  collapsed?: boolean;
  onToggle?: () => void;
}) {
  const t = useTranslations("shell");
  const slug = useSlug();
  const pathname = usePathname();
  const locale = useLocale();
  const qc = useQueryClient();
  const projects = useQuery({ queryKey: qk.projects, queryFn: api.projects });
  // Warm the project query on hover/focus so the overview renders without a skeleton.
  const prefetch = (s: string) => qc.prefetchQuery({ queryKey: qk.project(s), queryFn: () => api.project(s) });

  const item = (href: string, active: boolean) =>
    cn(
      "relative flex h-8 items-center gap-2 rounded-md px-2 text-sm pointer-coarse:h-10 text-muted-foreground transition-colors duration-150 hover:bg-accent hover:text-foreground [&_svg]:shrink-0",
      active &&
        "bg-accent font-medium text-foreground before:absolute before:inset-y-1.5 before:-start-2 before:w-0.5 before:rounded-full before:bg-brand",
    );
  const current = (active: boolean) => (active ? ("page" as const) : undefined);
  // Collapsed rail: icon only; the label stays for screen readers and shows as a tooltip.
  const label = (text: string) => <span className={collapsed ? "sr-only" : "truncate"}>{text}</span>;
  const tip = (text: string, node: React.ReactElement) =>
    collapsed ? (
      <Tooltip>
        <TooltipTrigger asChild>{node}</TooltipTrigger>
        <TooltipContent side={RTL_LOCALES.includes(locale) ? "left" : "right"}>{text}</TooltipContent>
      </Tooltip>
    ) : (
      node
    );

  return (
    <nav
      aria-label={t("mainNav")}
      className="flex h-full flex-col gap-6 overflow-x-hidden overflow-y-auto p-3"
      onClick={(e) => (e.target as HTMLElement).closest("a") && onNavigate?.()}
    >
      <div className={cn("flex items-center justify-between", !collapsed && "ps-2")}>
        {!collapsed && (
          <Link href="/app" className="pt-1">
            <Logo />
          </Link>
        )}
        {onToggle &&
          tip(
            t("expandSidebar"),
            <Button
              variant="ghost"
              size="icon-sm"
              className="text-muted-foreground"
              aria-label={collapsed ? t("expandSidebar") : t("collapseSidebar")}
              aria-expanded={!collapsed}
              aria-keyshortcuts="Meta+B Control+B"
              onClick={onToggle}
            >
              {collapsed ? <PanelLeftOpen className="rtl:-scale-x-100" /> : <PanelLeftClose className="rtl:-scale-x-100" />}
            </Button>,
          )}
      </div>

      <div>
        <div className={cn("mb-1 flex items-center justify-between", !collapsed && "px-2")}>
          {!collapsed && <span className="text-xs font-medium text-muted-foreground">{t("projects")}</span>}
          {tip(
            t("newProject"),
            <Button variant="ghost" size="icon-xs" aria-label={t("newProject")} onClick={onCreate} className={cn(collapsed && "mx-auto")}>
              <Plus />
            </Button>,
          )}
        </div>
        <div className="space-y-0.5">
          {projects.isPending &&
            Array.from({ length: 3 }, (_, i) => <Skeleton key={i} className="mx-2 my-1.5 h-5" />)}
          {projects.data?.length === 0 && <p className="px-2 py-1 text-xs text-muted-foreground">{t("noProjects")}</p>}
          {/* ponytail: the rail keeps only the open project; switching goes through ⌘K or expanding. */}
          {projects.data?.filter((p) => !collapsed || !slug || p.slug === slug).map((p) => (
            <div key={p.id}>
              {tip(
                p.name,
                <Link
                  href={`/app/${p.slug}`}
                  className={item(`/app/${p.slug}`, p.slug === slug && pathname === `/app/${p.slug}`)}
                  aria-current={current(p.slug === slug && pathname === `/app/${p.slug}`)}
                  onMouseEnter={() => prefetch(p.slug)}
                  onFocus={() => prefetch(p.slug)}
                >
                  <FolderKanban className="size-4" />
                  {label(p.name)}
                </Link>,
              )}
              {p.slug === slug && !collapsed && <SidebarProjectProgress slug={p.slug} />}
              {p.slug === slug && (
                <div className={cn("mt-2 space-y-0.5", collapsed ? "border-t pt-2" : "ms-4 border-s ps-2")}>
                  {PROJECT_PAGES.slice(1).map((page) => {
                    const { seg, icon: Icon, label: name } = page;
                    const href = `/app/${p.slug}${seg}`;
                    return (
                      <div key={seg}>
                        {seg === "/insights" &&
                          (collapsed ? (
                            <div role="separator" aria-label={t("labs")} className="my-2 border-t" />
                          ) : (
                            <div className="px-2 pt-3 pb-1 text-xs font-medium text-muted-foreground">{t("labs")}</div>
                          ))}
                        {tip(
                          t(name),
                          <Link href={href} className={item(href, pathname === href)} aria-current={current(pathname === href)}>
                            <Icon className="size-4" />
                            {label(t(name))}
                          </Link>,
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      <div className="mt-auto">
        {tip(
          t("account"),
          <Link href="/app/settings/account" className={item("", pathname === "/app/settings/account")} aria-current={current(pathname === "/app/settings/account")}>
            <UserRound className="size-4" />
            {label(t("account"))}
          </Link>,
        )}
        {tip(
          t("apiTokens"),
          <Link href="/app/settings/tokens" className={item("", pathname === "/app/settings/tokens")} aria-current={current(pathname === "/app/settings/tokens")}>
            <KeyRound className="size-4" />
            {label(t("apiTokens"))}
          </Link>,
        )}
        {tip(
          t("billing"),
          <Link href="/app/settings/billing" className={item("", pathname === "/app/settings/billing")} aria-current={current(pathname === "/app/settings/billing")}>
            <CreditCard className="size-4" />
            {label(t("billing"))}
          </Link>,
        )}
      </div>
    </nav>
  );
}

function CommandPalette({ open, setOpen, onCreate }: { open: boolean; setOpen: (v: boolean) => void; onCreate: () => void }) {
  const t = useTranslations("palette");
  const ts = useTranslations("shell");
  const router = useRouter();
  const slug = useSlug();
  const locale = useLocale();
  const name = useLocaleName();
  const switchLocale = useSwitchLocale();
  const { resolvedTheme, setTheme } = useTheme();
  const projects = useQuery({ queryKey: qk.projects, queryFn: api.projects });
  const translate = useTranslateMissing(slug);
  const [search, setSearch] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const id = setTimeout(() => setDebounced(search.trim()), 150);
    return () => clearTimeout(id);
  }, [search]);
  // "Go to key": server-side search over key names and source text of the current project.
  const keys = useQuery({
    queryKey: [...qk.keys(slug), "palette", debounced],
    queryFn: () => api.keys(slug, { q: debounced, per_page: 8 }),
    enabled: open && !!slug && debounced.length >= 2,
    staleTime: 30_000,
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen(!open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  const run = (fn: () => void) => {
    setOpen(false);
    fn();
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={(v) => {
        setOpen(v);
        if (!v) setSearch("");
      }}
      title={ts("search")}
      description={t("placeholder")}
    >
      <CommandInput placeholder={t("placeholder")} value={search} onValueChange={setSearch} />
      <CommandList>
        <CommandEmpty>{keys.isFetching ? t("searching") : t("empty")}</CommandEmpty>
        {!!keys.data?.items.length && (
          <CommandGroup heading={t("keys")}>
            {keys.data.items.map((k) => (
              <CommandItem
                key={k.key}
                // Matched server-side: include the query so cmdk's own filter keeps the item.
                value={`key ${k.key} ${k.source} ${search}`}
                onSelect={() => run(() => router.push(`/app/${slug}/keys?q=${encodeURIComponent(k.key)}`))}
              >
                <TableProperties />
                <span className="truncate font-mono text-xs">{k.key}</span>
                <span className="ms-auto max-w-[45%] truncate text-xs text-muted-foreground">{k.source}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        <CommandGroup heading={t("actions")}>
          {slug && (
            <CommandItem
              value={`action translate ${t("translateMissing")}`}
              disabled={translate.isPending}
              onSelect={() => run(() => translate.mutate())}
            >
              <Sparkles /> {t("translateMissing")}
            </CommandItem>
          )}
          <CommandItem value={`action create ${t("createProject")}`} onSelect={() => run(onCreate)}>
            <Plus /> {t("createProject")}
          </CommandItem>
        </CommandGroup>
        {slug && (
          <CommandGroup heading={t("navigation")}>
            {PROJECT_PAGES.map(({ seg, icon: Icon, label }) => (
              <CommandItem key={seg} onSelect={() => run(() => router.push(`/app/${slug}${seg}`))}>
                <Icon /> {t("goTo", { page: ts(label) })}
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        <CommandGroup heading={t("projects")}>
          {projects.data?.map((p) => (
            <CommandItem key={p.id} value={`${p.name} ${p.slug}`} onSelect={() => run(() => router.push(`/app/${p.slug}`))}>
              <FolderKanban /> {t("openProject", { name: p.name })}
            </CommandItem>
          ))}
          <CommandItem onSelect={() => run(() => router.push("/app/settings/tokens"))}>
            <KeyRound /> {t("goTo", { page: ts("apiTokens") })}
          </CommandItem>
          <CommandItem onSelect={() => run(() => router.push("/app/settings/billing"))}>
            <CreditCard /> {t("goTo", { page: ts("billing") })}
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading={t("preferences")}>
          <CommandItem onSelect={() => run(() => setTheme(resolvedTheme === "dark" ? "light" : "dark"))}>
            <Moon /> {t("toggleTheme")}
          </CommandItem>
          {routing.locales
            .filter((l) => l !== locale)
            .map((l) => (
              <CommandItem key={l} value={`language ${l} ${name(l, true)}`} onSelect={() => run(() => switchLocale(l))}>
                <Languages /> {t("switchLanguage", { language: name(l, true) })}
              </CommandItem>
            ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}

function UserMenu() {
  const t = useTranslations("shell");
  const router = useRouter();
  const qc = useQueryClient();
  const me = useQuery({ queryKey: qk.me, queryFn: api.me });
  const logout = useMutation({
    mutationFn: api.logout,
    onSettled: () => {
      qc.clear();
      router.replace("/login");
    },
  });
  const initials = (me.data?.name || me.data?.email || "?").slice(0, 2).toUpperCase();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" className="rounded-full">
          <Avatar className="size-7">
            <AvatarFallback className="text-[11px]">{me.isPending ? "" : initials}</AvatarFallback>
          </Avatar>
          <span className="sr-only">{t("userMenu")}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="font-normal">
          <div className="truncate text-sm font-medium">{me.data?.name}</div>
          <div className="truncate text-xs text-muted-foreground">{me.data?.email}</div>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link href="/app/settings/account">
            <UserRound /> {t("account")}
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link href="/app/settings/tokens">
            <KeyRound /> {t("apiTokens")}
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem asChild>
          <Link href="/app/settings/billing">
            <CreditCard /> {t("billing")}
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => logout.mutate()}>
          <LogOut /> {t("logout")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// Experimental-server notice: dismissible, remembered per browser.
const NOTE = "glot:experimental-note";
const noteListeners = new Set<() => void>();
function useNoteDismissed() {
  const dismissed = useSyncExternalStore(
    (cb) => (noteListeners.add(cb), () => noteListeners.delete(cb)),
    () => {
      // The notice is about self-hosting: never shown on the hosted cloud.
      if (location.hostname === "glotlang.com" || location.hostname.endsWith(".glotlang.com")) return true;
      try {
        return localStorage.getItem(NOTE) === "dismissed";
      } catch {
        return false;
      }
    },
    () => false,
  );
  const dismiss = () => {
    try {
      localStorage.setItem(NOTE, "dismissed");
    } catch {
      /* ignore */
    }
    noteListeners.forEach((l) => l());
  };
  return [dismissed, dismiss] as const;
}

// Desktop sidebar folded to an icon rail: remembered per browser, same pattern as the notice above.
const RAIL = "glot:sidebar-collapsed";
const railListeners = new Set<() => void>();
function useSidebarCollapsed() {
  const collapsed = useSyncExternalStore(
    (cb) => (railListeners.add(cb), () => railListeners.delete(cb)),
    () => {
      try {
        return localStorage.getItem(RAIL) === "1";
      } catch {
        return false;
      }
    },
    () => false,
  );
  const toggle = () => {
    try {
      localStorage.setItem(RAIL, collapsed ? "0" : "1");
    } catch {
      /* ignore */
    }
    railListeners.forEach((l) => l());
  };
  return [collapsed, toggle] as const;
}

// Full-bleed tools manage their own height; no banners above them.
const FULL_BLEED = /^\/app\/[^/]+\/(keys|preview)$/;

export function AppShell({ children }: { children: React.ReactNode }) {
  const t = useTranslations("shell");
  const tc = useTranslations("common");
  const slug = useSlug();
  const pathname = usePathname();
  const [noteDismissed, dismissNote] = useNoteDismissed();
  const fullBleed = FULL_BLEED.test(pathname);
  const locale = useLocale();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [collapsed, toggleCollapsed] = useSidebarCollapsed();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.isContentEditable;
      if (e.key.toLowerCase() === "b" && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && !typing) {
        e.preventDefault();
        toggleCollapsed();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleCollapsed]);
  // Validates the session early: a 401 here redirects to /login (see Providers).
  useQuery({ queryKey: qk.me, queryFn: api.me });

  return (
    <div className="flex min-h-screen">
      <a
        href="#main"
        className="sr-only z-50 rounded-md bg-background px-3 py-2 text-sm font-medium shadow-(--shadow-raised) focus:not-sr-only focus:fixed focus:start-3 focus:top-3"
      >
        {t("skipToContent")}
      </a>
      <aside
        className={cn(
          "sticky top-0 hidden h-screen shrink-0 border-e bg-sidebar motion-safe:transition-[width] motion-safe:duration-200 lg:block",
          collapsed ? "w-14" : "w-60",
        )}
      >
        <SidebarNav collapsed={collapsed} onToggle={toggleCollapsed} onCreate={() => setCreateOpen(true)} />
      </aside>
      <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
        {/* The drawer opens from the reading-start edge (right in Arabic), like the desktop sidebar. */}
        <SheetContent side={RTL_LOCALES.includes(locale) ? "right" : "left"} className="w-72 max-w-[85vw] p-0">
          <SheetTitle className="sr-only">{t("toggleSidebar")}</SheetTitle>
          <SidebarNav onNavigate={() => setMobileOpen(false)} onCreate={() => setCreateOpen(true)} />
        </SheetContent>
      </Sheet>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="sticky top-0 z-30 flex h-12 items-center gap-2 border-b bg-background/85 px-3 backdrop-blur sm:px-4">
          <Button variant="ghost" size="icon-sm" className="lg:hidden" aria-label={t("toggleSidebar")} onClick={() => setMobileOpen(true)}>
            <Menu />
          </Button>
          <button
            type="button"
            onClick={() => setPaletteOpen(true)}
            aria-keyshortcuts="Meta+K Control+K"
            className="flex h-8 w-full max-w-xs items-center gap-2 rounded-md border pointer-coarse:h-10 bg-muted/40 px-2.5 text-sm text-muted-foreground transition-colors duration-150 hover:bg-muted hover:text-foreground"
          >
            <Search className="size-4" aria-hidden />
            <span className="flex-1 text-start">{t("search")}</span>
            <kbd className="hidden rounded border bg-background px-1.5 font-mono text-[10px] sm:inline">⌘K</kbd>
          </button>
          <div className="ms-auto flex items-center gap-1">
            <LocaleSwitcher />
            <ThemeToggle />
            <UserMenu />
          </div>
        </header>
        <main id="main" tabIndex={-1} className="min-w-0 flex-1 px-4 py-6 outline-none sm:px-6 lg:px-8">
          {!fullBleed && !noteDismissed && (
            <div
              role="note"
              className="mb-6 flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 py-1.5 ps-3 pe-1.5 text-xs text-warning-foreground"
            >
              <p className="flex-1 py-1">
                {t.rich("experimental", {
                  docs: (c) => (
                    <Link href="/docs/self-hosting" className="font-medium underline underline-offset-2">
                      {c}
                    </Link>
                  ),
                })}
              </p>
              <Button variant="ghost" size="icon-xs" aria-label={tc("dismiss")} onClick={dismissNote} className="hover:bg-warning/15">
                <X />
              </Button>
            </div>
          )}
          {!fullBleed && <VerifyEmailBanner />}
          {/* Page transition: keyed by route so each navigation fades the content (see globals.css .page). */}
          <ViewTransition key={pathname} enter="page" exit="page" default="none">
            <div>{children}</div>
          </ViewTransition>
        </main>
      </div>

      <CommandPalette open={paletteOpen} setOpen={setPaletteOpen} onCreate={() => setCreateOpen(true)} />
      {slug && <ProjectCelebrations slug={slug} />}
      <CreateProjectDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  );
}
