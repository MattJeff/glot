"use client";

import { Search } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useMemo, useRef, useState } from "react";
import { Input } from "@/components/ui/input";
import { Link, usePathname } from "@/i18n/navigation";
import { cn } from "@/lib/utils";
import type { SearchEntry } from "./content";

type Nav = { section: string; title: string; pages: { slug: string; title: string }[] }[];

export function SidebarNav({ nav }: { nav: Nav }) {
  const pathname = usePathname();
  return (
    <nav className="space-y-6 text-sm">
      {nav.map((s) => (
        <div key={s.section}>
          <p className="mb-2 font-medium">{s.title}</p>
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
                      "-ms-px block border-s px-3 py-1 text-muted-foreground hover:border-foreground/40 hover:text-foreground",
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

/** Client-side full-text search over a static index: every query term must appear; heading hits rank first. */
export function DocsSearch({ index, titles }: { index: SearchEntry[]; titles: Record<string, string> }) {
  const t = useTranslations("docs.search");
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (e.key === "/" && !/^(INPUT|TEXTAREA)$/.test(el.tagName) && !el.isContentEditable) {
        e.preventDefault();
        input.current?.focus();
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
      .slice(0, 8);
  }, [q, index]);

  return (
    <div className="relative w-full max-w-sm">
      <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        ref={input}
        type="search"
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
        onKeyDown={(e) => e.key === "Escape" && (setQ(""), input.current?.blur())}
        placeholder={t("placeholder")}
        aria-label={t("label")}
        className="h-8 ps-8"
      />
      {open && q.trim() && (
        <div className="absolute inset-x-0 top-full z-50 mt-1 max-h-[70vh] overflow-auto rounded-md border bg-popover p-1 shadow-lg">
          {results.length === 0 ? (
            <p className="p-3 text-sm text-muted-foreground">{t("noResults")}</p>
          ) : (
            <ul lang="en">
              {results.map(({ e, snippet }) => (
                <li key={`${e.slug}#${e.anchor}`}>
                  <Link
                    href={`/docs/${e.slug}${e.anchor ? `#${e.anchor}` : ""}`}
                    onClick={() => setQ("")}
                    className="block rounded-sm px-2 py-1.5 hover:bg-accent"
                  >
                    <span className="block text-xs text-muted-foreground">{titles[e.slug]}</span>
                    <span className="block text-sm font-medium">{e.heading}</span>
                    <span className="line-clamp-2 block text-xs text-muted-foreground">{snippet}</span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
