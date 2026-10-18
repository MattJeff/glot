"use client";

import { Check, Search, X } from "lucide-react";
import { useTranslations } from "next-intl";
import { useId, useState } from "react";
import { useLocaleName } from "@/components/common";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { canonical, flag, POPULAR } from "./locales";

/** Searchable language chips: native radios (single) or checkboxes (multi), names in the UI language. */
export function LocalePicker({
  mode,
  label,
  value,
  onChange,
  exclude,
  detected,
}: {
  mode: "single" | "multi";
  label: string;
  value: string[];
  onChange: (v: string[]) => void;
  exclude?: string;
  detected?: string;
}) {
  const t = useTranslations("projects.create.picker");
  const name = useLocaleName();
  const id = useId();
  const [q, setQ] = useState("");
  const query = q.trim().toLowerCase();

  const base = [...new Set([...(detected ? [detected] : []), ...value, ...POPULAR])].filter((l) => l !== exclude);
  const matches = (l: string) =>
    l.toLowerCase().startsWith(query) || name(l).toLowerCase().includes(query) || name(l, true).toLowerCase().includes(query);
  let options = query ? base.filter(matches) : base.slice(0, 12 + value.length);
  const custom = query ? canonical(q) : null;
  if (custom && custom !== exclude && !options.includes(custom)) options = [custom, ...options];
  options = [...new Set(options)];

  const toggle = (l: string) =>
    mode === "single" ? onChange([l]) : onChange(value.includes(l) ? value.filter((x) => x !== l) : [...value, l]);

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <label htmlFor={`${id}-q`} className="text-sm font-medium">
          {label}
        </label>
        <div className="relative">
          <Search aria-hidden className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            id={`${id}-q`}
            type="search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              // Enter picks the first match instead of submitting the step.
              if (e.key === "Enter" && query && options[0]) {
                e.preventDefault();
                if (!value.includes(options[0]) || mode === "single") toggle(options[0]);
                setQ("");
              }
            }}
            placeholder={t("search")}
            aria-describedby={`${id}-hint`}
            className="ps-8"
            autoComplete="off"
          />
        </div>
        <p id={`${id}-hint`} className="text-xs text-muted-foreground">
          {mode === "multi" ? t("multiHint") : t("singleHint")}
        </p>
      </div>

      {mode === "multi" && (
        <div className="flex min-h-7 flex-wrap items-center gap-1.5">
          <span role="status" aria-live="polite" className="me-1 text-xs text-muted-foreground tabular-nums">
            {t("selected", { count: value.length })}
          </span>
          {value.map((l) => (
            <button
              key={l}
              type="button"
              onClick={() => toggle(l)}
              className="inline-flex h-7 items-center gap-1 rounded-full border bg-secondary px-2.5 text-xs transition-colors duration-150 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none animate-in zoom-in-95 duration-150 motion-reduce:animate-none"
              aria-label={t("remove", { language: name(l) })}
            >
              <span aria-hidden>{flag(l)}</span>
              <span className="capitalize">{name(l)}</span>
              <X aria-hidden className="size-3" />
            </button>
          ))}
        </div>
      )}

      <fieldset>
        <legend className="sr-only">{label}</legend>
        {options.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("noMatch")}</p>
        ) : (
          <div className="grid max-h-60 grid-cols-1 gap-1.5 overflow-y-auto p-0.5 sm:grid-cols-2">
            {options.map((l) => {
              const checked = value.includes(l);
              return (
                <label key={l} className="relative block cursor-pointer">
                  <input
                    type={mode === "single" ? "radio" : "checkbox"}
                    name={`${id}-locale`}
                    value={l}
                    checked={checked}
                    onChange={() => toggle(l)}
                    className="peer sr-only"
                  />
                  <span className="flex min-h-10 items-center gap-2 rounded-md border px-2.5 py-1.5 text-sm transition-colors duration-150 hover:bg-accent/50 peer-checked:border-brand peer-checked:bg-brand/5 peer-focus-visible:ring-2 peer-focus-visible:ring-ring peer-focus-visible:ring-offset-1 peer-focus-visible:ring-offset-background">
                    <span aria-hidden className="text-base leading-none">
                      {flag(l)}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate capitalize">{name(l)}</span>
                      {l === detected && <span className="block text-[11px] text-muted-foreground">{t("detected")}</span>}
                    </span>
                    <span dir="ltr" className="font-mono text-[11px] text-muted-foreground">
                      {l}
                    </span>
                    <Check aria-hidden className={cn("size-4 text-brand", !checked && "invisible")} />
                  </span>
                </label>
              );
            })}
          </div>
        )}
      </fieldset>
    </div>
  );
}
