"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  BookOpenText,
  FolderKanban,
  Grid3x3,
  KeyRound,
  LayoutDashboard,
  ListChecks,
  LogOut,
  Menu,
  Moon,
  Plus,
  ScanEye,
  Search,
  Settings,
  TableProperties,
  Languages,
} from "lucide-react";
import { useLocale, useTranslations } from "next-intl";
import { useTheme } from "next-themes";
import { useEffect, useState } from "react";
import { Logo, LocaleSwitcher, ThemeToggle, useLocaleName, useSwitchLocale } from "@/components/common";
import { CreateProjectDialog } from "@/components/create-project-dialog";
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
import { Link, usePathname, useRouter } from "@/i18n/navigation";
import { routing } from "@/i18n/routing";
import { api, qk } from "@/lib/api";
import { useSlug } from "@/lib/hooks";
import { cn } from "@/lib/utils";

const PROJECT_PAGES = [
  { seg: "", icon: LayoutDashboard, label: "overview" },
  { seg: "/keys", icon: TableProperties, label: "keys" },
  { seg: "/preview", icon: Grid3x3, label: "preview" },
  { seg: "/visual", icon: ScanEye, label: "visual" },
  { seg: "/glossary", icon: BookOpenText, label: "glossary" },
  { seg: "/jobs", icon: ListChecks, label: "jobs" },
  { seg: "/settings", icon: Settings, label: "settings" },
] as const;

function SidebarNav({ onNavigate, onCreate }: { onNavigate?: () => void; onCreate: () => void }) {
  const t = useTranslations("shell");
  const slug = useSlug();
  const pathname = usePathname();
  const projects = useQuery({ queryKey: qk.projects, queryFn: api.projects });

  const item = (href: string, active: boolean) =>
    cn(
      "flex h-8 items-center gap-2 rounded-md px-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
      active && "bg-accent font-medium text-foreground",
    );

  return (
    <nav className="flex h-full flex-col gap-6 p-3" onClick={(e) => (e.target as HTMLElement).closest("a") && onNavigate?.()}>
      <Link href="/app" className="px-2 pt-1">
        <Logo />
      </Link>

      <div>
        <div className="mb-1 flex items-center justify-between px-2">
          <span className="text-xs font-medium text-muted-foreground">{t("projects")}</span>
          <Button variant="ghost" size="icon-xs" aria-label={t("newProject")} onClick={onCreate}>
            <Plus />
          </Button>
        </div>
        <div className="space-y-0.5">
          {projects.isPending &&
            Array.from({ length: 3 }, (_, i) => <Skeleton key={i} className="mx-2 my-1.5 h-5" />)}
          {projects.data?.length === 0 && <p className="px-2 py-1 text-xs text-muted-foreground">{t("noProjects")}</p>}
          {projects.data?.map((p) => (
            <div key={p.id}>
              <Link href={`/app/${p.slug}`} className={item(`/app/${p.slug}`, p.slug === slug && pathname === `/app/${p.slug}`)}>
                <FolderKanban className="size-4" />
                <span className="truncate">{p.name}</span>
              </Link>
              {p.slug === slug && (
                <div className="ms-4 mt-0.5 space-y-0.5 border-s ps-2">
                  {PROJECT_PAGES.slice(1).map(({ seg, icon: Icon, label }) => {
                    const href = `/app/${p.slug}${seg}`;
                    return (
                      <Link key={seg} href={href} className={item(href, pathname === href)}>
                        <Icon className="size-4" />
                        {t(label)}
                      </Link>
                    );
                  })}
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      <div className="mt-auto">
        <Link href="/app/settings/tokens" className={item("", pathname === "/app/settings/tokens")}>
          <KeyRound className="size-4" />
          {t("apiTokens")}
        </Link>
      </div>
    </nav>
  );
}

function CommandPalette({ open, setOpen }: { open: boolean; setOpen: (v: boolean) => void }) {
  const t = useTranslations("palette");
  const ts = useTranslations("shell");
  const router = useRouter();
  const slug = useSlug();
  const locale = useLocale();
  const name = useLocaleName();
  const switchLocale = useSwitchLocale();
  const { resolvedTheme, setTheme } = useTheme();
  const projects = useQuery({ queryKey: qk.projects, queryFn: api.projects });

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
    <CommandDialog open={open} onOpenChange={setOpen} title={ts("search")} description={t("placeholder")}>
      <CommandInput placeholder={t("placeholder")} />
      <CommandList>
        <CommandEmpty>{t("empty")}</CommandEmpty>
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
        <Button variant="ghost" size="icon-sm" className="rounded-full" aria-label={t("userMenu")}>
          <Avatar className="size-7">
            <AvatarFallback className="text-[11px]">{me.isPending ? "" : initials}</AvatarFallback>
          </Avatar>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="font-normal">
          <div className="truncate text-sm font-medium">{me.data?.name}</div>
          <div className="truncate text-xs text-muted-foreground">{me.data?.email}</div>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link href="/app/settings/tokens">
            <KeyRound /> {t("apiTokens")}
          </Link>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => logout.mutate()}>
          <LogOut /> {t("logout")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const t = useTranslations("shell");
  const [mobileOpen, setMobileOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  // Validates the session early: a 401 here redirects to /login (see Providers).
  useQuery({ queryKey: qk.me, queryFn: api.me });

  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 hidden h-screen w-60 shrink-0 border-e bg-sidebar lg:block">
        <SidebarNav onCreate={() => setCreateOpen(true)} />
      </aside>
      <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
        <SheetContent side="left" className="w-64 p-0">
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
            className="flex h-8 w-full max-w-xs items-center gap-2 rounded-md border bg-muted/40 px-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted"
          >
            <Search className="size-4" />
            <span className="flex-1 text-start">{t("search")}</span>
            <kbd className="hidden rounded border bg-background px-1.5 font-mono text-[10px] sm:inline">⌘K</kbd>
          </button>
          <div className="ms-auto flex items-center gap-1">
            <LocaleSwitcher />
            <ThemeToggle />
            <UserMenu />
          </div>
        </header>
        <main id="main" className="min-w-0 flex-1 px-4 py-6 sm:px-6 lg:px-8">
          {children}
        </main>
      </div>

      <CommandPalette open={paletteOpen} setOpen={setPaletteOpen} />
      <CreateProjectDialog open={createOpen} onOpenChange={setCreateOpen} />
    </div>
  );
}
