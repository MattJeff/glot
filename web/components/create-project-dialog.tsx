"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useRouter } from "@/i18n/navigation";
import { api, qk } from "@/lib/api";

export const slugify = (s: string) =>
  s
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 48);

export const parseLocales = (s: string) =>
  [...new Set(s.split(/[\s,]+/).map((l) => l.trim()).filter(Boolean))];

export function CreateProjectDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const t = useTranslations("projects.create");
  const tc = useTranslations("common");
  const qc = useQueryClient();
  const router = useRouter();
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [slugTouched, setSlugTouched] = useState(false);

  const create = useMutation({
    mutationFn: api.createProject,
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: qk.projects });
      toast.success(t("success", { name: p.name }));
      onOpenChange(false);
      setName("");
      setSlug("");
      setSlugTouched(false);
      router.push(`/app/${p.slug}`);
    },
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            create.mutate({
              name,
              slug,
              source_locale: String(f.get("source_locale")).trim(),
              target_locales: parseLocales(String(f.get("target_locales"))),
            });
          }}
          className="space-y-4"
        >
          <DialogHeader>
            <DialogTitle>{t("title")}</DialogTitle>
            <DialogDescription>{t("description")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="p-name">{t("name")}</Label>
            <Input
              id="p-name"
              required
              value={name}
              placeholder={t("namePlaceholder")}
              onChange={(e) => {
                setName(e.target.value);
                if (!slugTouched) setSlug(slugify(e.target.value));
              }}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="p-slug">{t("slug")}</Label>
            <Input
              id="p-slug"
              required
              dir="ltr"
              pattern="[a-z0-9][a-z0-9\-]*"
              className="font-mono"
              value={slug}
              aria-describedby="p-slug-hint"
              onChange={(e) => {
                setSlugTouched(true);
                setSlug(e.target.value);
              }}
            />
            <p id="p-slug-hint" className="text-xs text-muted-foreground">{t("slugHint")}</p>
          </div>
          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-1.5">
              <Label htmlFor="p-src">{t("sourceLocale")}</Label>
              <Input id="p-src" name="source_locale" required dir="ltr" defaultValue="en" className="font-mono" />
            </div>
            <div className="col-span-2 space-y-1.5">
              <Label htmlFor="p-targets">{t("targetLocales")}</Label>
              <Input id="p-targets" name="target_locales" required dir="ltr" defaultValue="fr, de, es" className="font-mono" aria-describedby="p-targets-hint" />
            </div>
          </div>
          <p id="p-targets-hint" className="-mt-2 text-xs text-muted-foreground">{t("targetLocalesHint")}</p>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {tc("cancel")}
            </Button>
            <Button type="submit" disabled={create.isPending}>
              {create.isPending && <Loader2 className="animate-spin" />}
              {t("submit")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
