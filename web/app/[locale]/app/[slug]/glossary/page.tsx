"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpenText, Loader2, Pencil, Plus, Trash2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { EmptyState, ErrorState, PageHeader, TermTip } from "@/components/common";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { api, qk, type Term, type TermInput } from "@/lib/api";
import { useProject, useSlug } from "@/lib/hooks";

export default function GlossaryPage() {
  const t = useTranslations("glossary");
  const tc = useTranslations("common");
  const slug = useSlug();
  const qc = useQueryClient();
  const project = useProject(slug);
  const terms = useQuery({ queryKey: qk.glossary(slug), queryFn: () => api.glossary(slug) });
  const [editing, setEditing] = useState<Term | "new" | null>(null);
  const [deleting, setDeleting] = useState<Term | null>(null);
  const locales = project.data?.target_locales ?? [];

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteTerm(slug, id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.glossary(slug) });
      toast.success(t("deleted"));
      setDeleting(null);
    },
  });

  const addButton = (
    <Button onClick={() => setEditing("new")}>
      <Plus /> {t("add")}
    </Button>
  );

  return (
    <div className="mx-auto max-w-6xl">
      <PageHeader
        title={t("title")}
        subtitle={t.rich("subtitle", { file: (c) => <TermTip tip={t("fileTip")}>{c}</TermTip> })} actions={terms.data?.length ? addButton : null} />
      {terms.isPending ? (
        <Skeleton className="h-64 rounded-xl" />
      ) : terms.isError ? (
        <ErrorState error={terms.error} onRetry={() => terms.refetch()} />
      ) : terms.data.length === 0 ? (
        <EmptyState icon={BookOpenText} title={t("empty.title")} body={t("empty.body")} action={addButton} />
      ) : (
        <div className="overflow-x-auto rounded-xl border bg-card shadow-(--shadow-card)">
          <table className="w-full min-w-[640px] text-sm">
            <thead className="border-b bg-muted/40 text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="px-4 py-2 text-start font-medium">{t("term")}</th>
                <th scope="col" className="px-4 py-2 text-start font-medium">{t("translations")}</th>
                <th scope="col" className="px-4 py-2 text-start font-medium">{t("flags")}</th>
                <th scope="col" className="w-20 px-4 py-2"><span className="sr-only">{tc("edit")}</span></th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {terms.data.map((term) => (
                <tr key={term.id} className="align-top hover:bg-accent/30">
                  <td className="px-4 py-3">
                    <div className="font-medium">{term.term}</div>
                    {term.description && <div className="mt-0.5 text-xs text-muted-foreground">{term.description}</div>}
                  </td>
                  <td className="px-4 py-3">
                    {term.do_not_translate ? (
                      <span className="text-xs text-muted-foreground">—</span>
                    ) : (
                      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
                        {Object.entries(term.translations).map(([l, v]) => (
                          <div key={l} className="contents">
                            <dt className="font-mono text-muted-foreground">{l}</dt>
                            <dd dir="auto" lang={l}>
                              {v}
                              {term.forbidden[l]?.length ? (
                                <span className="ms-2 text-destructive line-through decoration-destructive/60">
                                  {term.forbidden[l].join(", ")}
                                </span>
                              ) : null}
                            </dd>
                          </div>
                        ))}
                      </dl>
                    )}
                  </td>
                  <td className="px-4 py-3">
                    <div className="flex flex-wrap gap-1">
                      {term.do_not_translate && <Badge variant="secondary">{t("doNotTranslate")}</Badge>}
                      {term.case_sensitive && <Badge variant="outline">{t("caseSensitive")}</Badge>}
                    </div>
                  </td>
                  <td className="px-2 py-2">
                    <div className="flex justify-end gap-0.5">
                      <Button variant="ghost" size="icon-sm" aria-label={tc("edit")} onClick={() => setEditing(term)}>
                        <Pencil />
                      </Button>
                      <Button variant="ghost" size="icon-sm" aria-label={tc("delete")} onClick={() => setDeleting(term)}>
                        <Trash2 />
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {editing && (
        <TermDialog
          slug={slug}
          locales={locales}
          term={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
        />
      )}

      <AlertDialog open={!!deleting} onOpenChange={(o) => !o && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("deleteConfirm.title", { term: deleting?.term ?? "" })}</AlertDialogTitle>
            <AlertDialogDescription>{t("deleteConfirm.body")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{tc("cancel")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => deleting && remove.mutate(deleting.id)}>
              {tc("delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function TermDialog({ slug, locales, term, onClose }: { slug: string; locales: string[]; term: Term | null; onClose: () => void }) {
  const t = useTranslations("glossary");
  const tc = useTranslations("common");
  const qc = useQueryClient();
  const [dnt, setDnt] = useState(term?.do_not_translate ?? false);
  const [caseSensitive, setCaseSensitive] = useState(term?.case_sensitive ?? false);

  const save = useMutation({
    mutationFn: (b: TermInput) => (term ? api.updateTerm(slug, term.id, b) : api.createTerm(slug, b)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.glossary(slug) });
      toast.success(t("saved"));
      onClose();
    },
  });

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
        <form
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            const get = (k: string) => String(f.get(k) ?? "").trim();
            const translations: Record<string, string> = {};
            const forbidden: Record<string, string[]> = {};
            for (const l of locales) {
              if (get(`tr-${l}`)) translations[l] = get(`tr-${l}`);
              const fb = get(`fb-${l}`).split(",").map((x) => x.trim()).filter(Boolean);
              if (fb.length) forbidden[l] = fb;
            }
            save.mutate({
              term: get("term"),
              description: get("description") || null,
              do_not_translate: dnt,
              case_sensitive: caseSensitive,
              translations: dnt ? {} : translations,
              forbidden: dnt ? {} : forbidden,
            });
          }}
        >
          <DialogHeader>
            <DialogTitle>{term ? t("dialog.editTitle") : t("dialog.createTitle")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="g-term">{t("term")}</Label>
            <Input id="g-term" name="term" required defaultValue={term?.term} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="g-desc">{t("description")}</Label>
            <Textarea id="g-desc" name="description" rows={2} defaultValue={term?.description ?? ""} />
          </div>
          <div className="flex flex-wrap gap-6">
            <label className="flex items-center gap-2 text-sm">
              <Switch checked={dnt} onCheckedChange={setDnt} /> {t("doNotTranslate")}
            </label>
            <label className="flex items-center gap-2 text-sm">
              <Switch checked={caseSensitive} onCheckedChange={setCaseSensitive} /> {t("caseSensitive")}
            </label>
          </div>
          {!dnt && (
            <div className="space-y-3 rounded-lg border p-3">
              {locales.map((l) => (
                <div key={l} className="grid gap-2 sm:grid-cols-2">
                  <div className="space-y-1">
                    <Label htmlFor={`tr-${l}`} className="text-xs">{t("dialog.translationFor", { locale: l })}</Label>
                    <Input id={`tr-${l}`} name={`tr-${l}`} lang={l} dir="auto" defaultValue={term?.translations[l]} />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor={`fb-${l}`} className="text-xs">{t("dialog.forbiddenFor", { locale: l })}</Label>
                    <Input
                      id={`fb-${l}`}
                      name={`fb-${l}`}
                      lang={l}
                      dir="auto"
                      defaultValue={term?.forbidden[l]?.join(", ")}
                      title={t("forbiddenHint")}
                    />
                  </div>
                </div>
              ))}
              <p className="text-xs text-muted-foreground">{t("forbiddenHint")}</p>
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              {tc("cancel")}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {save.isPending && <Loader2 className="animate-spin" />}
              {tc("save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
