"use client";

import { Search } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Link } from "@/i18n/navigation";
import { cn } from "@/lib/utils";
import type { Category, Method } from "./data";

type Item = { id: string; name: string; category: Category; method: Method };

/** Filterable grid. Server-rendered with every card, so it works (and is crawled) without JavaScript. */
export function IntegrationsGrid({ items, categories }: { items: Item[]; categories: readonly Category[] }) {
  const t = useTranslations("integrations");
  const [q, setQ] = useState("");
  const [cat, setCat] = useState<Category | null>(null);
  const needle = q.trim().toLowerCase();
  const shown = items.filter((i) => (!cat || i.category === cat) && (!needle || i.name.toLowerCase().includes(needle)));
  const chip = (active: boolean) =>
    cn(
      "rounded-full border px-3 py-1 text-sm transition-colors",
      active ? "border-foreground bg-foreground text-background" : "text-muted-foreground hover:bg-muted hover:text-foreground",
    );

  return (
    <div>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative sm:w-72">
          <Search aria-hidden className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder={t("searchPlaceholder")}
            aria-label={t("search")}
            className="ps-8"
          />
        </div>
        <div role="group" aria-label={t("filter")} className="flex flex-wrap gap-2">
          <button type="button" aria-pressed={!cat} className={chip(!cat)} onClick={() => setCat(null)}>
            {t("all")}
          </button>
          {categories.map((c) => (
            <button key={c} type="button" aria-pressed={cat === c} className={chip(cat === c)} onClick={() => setCat(c)}>
              {t(`categories.${c}`)}
            </button>
          ))}
        </div>
      </div>
      <p aria-live="polite" className="mt-4 text-sm text-muted-foreground">
        {t("count", { count: shown.length })}
      </p>
      {shown.length ? (
        <ul className="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {shown.map((i) => (
            <li key={i.id}>
              <Link
                href={`/integrations/${i.id}`}
                className="flex h-full flex-col gap-1 rounded-xl border p-4 transition-colors hover:border-foreground/30 hover:bg-muted/40"
              >
                <span className="font-medium">{i.name}</span>
                <span className="text-sm text-muted-foreground">
                  {t(`categories.${i.category}`)} · {t(`methods.${i.method}`)}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-6 rounded-xl border border-dashed p-6 text-sm text-muted-foreground">{t("empty", { q: q.trim() })}</p>
      )}
    </div>
  );
}
