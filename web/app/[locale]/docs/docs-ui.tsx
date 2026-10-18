"use client";

import { FileText, Hash, Search } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { Command, CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Link, usePathname, useRouter } from "@/i18n/navigation";
import { cn } from "@/lib/utils";
import type { SearchEntry } from "./content";

type Nav = { section: string; title: string; pages: { slug: string; title: string }[] }[];

export function SidebarNav({ nav }: { nav: Nav }) {
  const pathname = usePathname();
  return (
    <nav className="space-y-6 text-sm">
      {nav.map((s) => (
        <div key={s.section}>
          <p className="mb-2 text-xs font-semibold tracking-wide text-foreground/80 uppercase">{s.title}</p>
          <ul className="space-y-0.5 border-s">
            {s.pages.map((p) => {
              const href = `/docs/${p.slug}`;
              const active = pathname === href;
              return (
                <li key={p.slug}>
                  <Link
                    href={href}
                    // Closes the mobile <details> menu after a click.
                    onClick={(e) => e.currentTarget.closest("details")?.removeAttribute("open")}
                    aria-current={active ? "page" : undefined}
                    className={cn(
                      "-ms-px block border-s px-3 py-1.5 text-muted-foreground transition-colors hover:border-foreground/40 hover:text-foreground",
                      active && "border-brand font-medium text-foreground",
                    )}
                  >
                    {p.title}
                  </Link>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </nav>
  );
}

const noop = () => () => {};

/** ⌘K / Ctrl+K / "/" search over a static index: every query term must appear; heading hits rank first. */
export function DocsSearch({ index, titles }: { index: SearchEntry[]; titles: Record<string, string> }) {
  const t = useTranslations("docs.search");
  const router = useRouter();
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(false);
  const mac = useSyncExternalStore(noop, () => /Mac|iPhone|iPad/.test(navigator.platform), () => true);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.isContentEditable;
      if ((e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && !typing)) {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const results = useMemo(() => {
    const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length) return [];
    return index
      .map((e) => {
        const head = e.heading.toLowerCase();
        const body = e.text.toLowerCase();
        if (!terms.every((w) => head.includes(w) || body.includes(w))) return null;
        const score = terms.reduce((n, w) => n + (head.includes(w) ? 10 : 0) + (body.includes(w) ? 1 : 0), 0);
        const at = Math.max(0, body.indexOf(terms[0]) - 40);
        return { e, score, snippet: (at ? "…" : "") + e.text.slice(at, at + 140) };
      })
      .filter((r) => r !== null)
      .sort((a, b) => b.score - a.score)
      .slice(0, 10);
  }, [q, index]);

  const pages = Object.entries(titles);
  const go = (href: string) => {
    setOpen(false);
    setQ("");
    router.push(href);
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-keyshortcuts="Meta+K Control+K /"
        className="flex h-9 w-full max-w-xs items-center gap-2 rounded-md border bg-muted/40 px-3 text-sm text-muted-foreground shadow-xs transition-colors hover:bg-muted hover:text-foreground"
      >
        <Search aria-hidden className="size-4" />
        <span className="flex-1 truncate text-start">{t("placeholder")}</span>
        <kbd aria-hidden className="hidden rounded border bg-background px-1.5 font-mono text-[11px] sm:inline">
          {mac ? "⌘K" : "Ctrl K"}
        </kbd>
      </button>
      <CommandDialog open={open} onOpenChange={setOpen} title={t("label")} description={t("hint")}>
        <Command shouldFilter={false}>
        <CommandInput value={q} onValueChange={setQ} placeholder={t("input")} />
        <CommandList className="max-h-[60vh]">
          {q.trim() ? (
            <>
              <CommandEmpty>{t("noResults")}</CommandEmpty>
              {results.map(({ e, snippet }) => {
                const href = `/docs/${e.slug}${e.anchor ? `#${e.anchor}` : ""}`;
                return (
                  <CommandItem key={href} value={href} onSelect={() => go(href)} className="items-start gap-3 py-2">
                    <Hash aria-hidden className="mt-0.5 size-4 shrink-0" />
                    <span className="min-w-0" lang="en">
                      <span className="block text-xs text-muted-foreground">{titles[e.slug]}</span>
                      <span className="block font-medium">{e.heading}</span>
                      <span className="line-clamp-2 block text-xs text-muted-foreground">{snippet}</span>
                    </span>
                  </CommandItem>
                );
              })}
            </>
          ) : (
            <CommandGroup heading={t("pages")}>
              {pages.map(([slug, title]) => (
                <CommandItem key={slug} value={slug} onSelect={() => go(`/docs/${slug}`)}>
                  <FileText aria-hidden className="size-4" /> {title}
                </CommandItem>
              ))}
            </CommandGroup>
          )}
        </CommandList>
        </Command>
      </CommandDialog>
    </>
  );
}

/** "On this page" with the section currently being read highlighted. */
export function Toc({ items, label }: { items: { id: string; text: string }[]; label: string }) {
  const [active, setActive] = useState(items[0]?.id);
  useEffect(() => {
    // The current section = the last heading that scrolled past the header.
    let raf = 0;
    const onScroll = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        let cur = items[0]?.id;
        for (const i of items) {
          const el = document.getElementById(i.id);
          if (el && el.getBoundingClientRect().top < 120) cur = i.id;
        }
        setActive(cur);
      });
    };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("scroll", onScroll);
    };
  }, [items]);
  return (
    <nav aria-label={label} className="text-sm">
      <p className="mb-3 text-xs font-semibold tracking-wide text-foreground/80 uppercase">{label}</p>
      <ul lang="en" className="space-y-0.5 border-s">
        {items.map((h) => (
          <li key={h.id}>
            <a
              href={`#${h.id}`}
              aria-current={active === h.id ? "location" : undefined}
              className={cn(
                "-ms-px block border-s py-1 ps-3 text-muted-foreground transition-colors hover:text-foreground",
                active === h.id && "border-brand text-foreground",
              )}
            >
              {h.text}
            </a>
          </li>
        ))}
      </ul>
    </nav>
  );
}

/** Adds a copy button to every code block of the rendered Markdown. */
export function CodeCopy({ copy, copied }: { copy: string; copied: string }) {
  useEffect(() => {
    const pres = document.querySelectorAll<HTMLPreElement>(".docs-prose pre");
    const added: HTMLButtonElement[] = [];
    pres.forEach((pre) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "copy-btn";
      b.textContent = copy;
      b.onclick = async () => {
        try {
          await navigator.clipboard.writeText(pre.querySelector("code")?.innerText ?? pre.innerText);
          b.textContent = copied;
          b.dataset.done = "";
          setTimeout(() => {
            b.textContent = copy;
            delete b.dataset.done;
          }, 1500);
        } catch {
          // Clipboard blocked: the code stays selectable.
        }
      };
      // Wrapper so the button stays put while the code scrolls horizontally.
      if (!pre.parentElement?.classList.contains("code-wrap")) {
        const w = document.createElement("div");
        w.className = "code-wrap";
        pre.before(w);
        w.append(pre);
      }
      pre.parentElement!.append(b);
      added.push(b);
    });
    return () => added.forEach((b) => b.remove());
  }, [copy, copied]);
  return null;
}
