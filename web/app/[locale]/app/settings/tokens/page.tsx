"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Loader2, Plus, Trash2 } from "lucide-react";
import { useFormatter, useNow, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { CodeBlock, CopyButton, EmptyState, ErrorState, PageHeader } from "@/components/common";
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
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type ApiToken, qk } from "@/lib/api";
import { useOrigin } from "@/lib/hooks";
import { cliLogin, mcpJson } from "@/lib/snippets";

export default function TokensPage() {
  const t = useTranslations("tokens");
  const tc = useTranslations("common");
  const format = useFormatter();
  const now = useNow({ updateInterval: 60_000 });
  const qc = useQueryClient();
  const tokens = useQuery({ queryKey: qk.tokens, queryFn: api.tokens });
  const [createOpen, setCreateOpen] = useState(false);
  const [secret, setSecret] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<ApiToken | null>(null);
  const origin = useOrigin();

  const create = useMutation({
    mutationFn: api.createToken,
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: qk.tokens });
      setCreateOpen(false);
      setSecret(r.token);
    },
  });
  const revoke = useMutation({
    mutationFn: api.deleteToken,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.tokens });
      toast.success(t("revoked"));
      setRevoking(null);
    },
  });

  const date = (d: string) => format.dateTime(new Date(d), { dateStyle: "medium" });

  return (
    <div className="mx-auto max-w-4xl">
      <PageHeader
        title={t("title")}
        subtitle={t("subtitle")}
        actions={
          <Button onClick={() => setCreateOpen(true)}>
            <Plus /> {t("create")}
          </Button>
        }
      />

      {tokens.isPending ? (
        <Skeleton className="h-40 rounded-xl" />
      ) : tokens.isError ? (
        <ErrorState error={tokens.error} onRetry={() => tokens.refetch()} />
      ) : tokens.data.length === 0 ? (
        <EmptyState icon={KeyRound} title={t("empty.title")} body={t("empty.body")} />
      ) : (
        <div className="overflow-x-auto rounded-xl border">
          <table className="w-full text-sm">
            <thead className="border-b bg-muted/40 text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="px-4 py-2 text-start font-medium">{t("name")}</th>
                <th scope="col" className="px-4 py-2 text-start font-medium">{t("created")}</th>
                <th scope="col" className="px-4 py-2 text-start font-medium">{t("lastUsed")}</th>
                <th scope="col" className="w-12"><span className="sr-only">{t("revoke")}</span></th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {tokens.data.map((tok) => (
                <tr key={tok.id}>
                  <td className="px-4 py-2.5 font-medium">{tok.name}</td>
                  <td className="px-4 py-2.5 text-muted-foreground">{date(tok.created_at)}</td>
                  <td className="px-4 py-2.5 text-muted-foreground">
                    {tok.last_used_at ? format.relativeTime(new Date(tok.last_used_at), now) : tc("never")}
                  </td>
                  <td className="px-2">
                    <Button variant="ghost" size="icon-sm" aria-label={t("revoke")} onClick={() => setRevoking(tok)}>
                      <Trash2 />
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <section className="mt-10">
        <h2 className="text-base font-semibold">{t("setup.title")}</h2>
        <div className="mt-4 grid gap-4 lg:grid-cols-2">
          <div>
            <p className="mb-2 text-sm text-muted-foreground">{t("setup.cliHint")}</p>
            <CodeBlock label={t("setup.cli")} code={cliLogin(origin)} />
          </div>
          <div>
            <p className="mb-2 text-sm text-muted-foreground">{t("setup.mcpHint")}</p>
            <CodeBlock label={t("setup.mcp")} code={mcpJson(secret ?? undefined, origin)} />
          </div>
        </div>
      </section>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="sm:max-w-md">
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              create.mutate(String(new FormData(e.currentTarget).get("name")).trim());
            }}
          >
            <DialogHeader>
              <DialogTitle>{t("create")}</DialogTitle>
              <DialogDescription>{t("subtitle")}</DialogDescription>
            </DialogHeader>
            <div className="space-y-1.5">
              <Label htmlFor="tok-name">{t("name")}</Label>
              <Input id="tok-name" name="name" required placeholder={t("namePlaceholder")} />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
                {tc("cancel")}
              </Button>
              <Button type="submit" disabled={create.isPending}>
                {create.isPending && <Loader2 className="animate-spin" />}
                {tc("create")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={!!secret} onOpenChange={(o) => !o && setSecret(null)}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("reveal.title")}</DialogTitle>
            <DialogDescription>{t("reveal.body")}</DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-2 rounded-lg border bg-muted/40 p-2 ps-3">
            <code dir="ltr" className="flex-1 font-mono text-sm break-all">{secret}</code>
            <CopyButton value={secret ?? ""} />
          </div>
          <CodeBlock label={t("setup.mcp")} code={mcpJson(secret ?? undefined, origin)} />
          <DialogFooter>
            <Button onClick={() => setSecret(null)}>{tc("close")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={!!revoking} onOpenChange={(o) => !o && setRevoking(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("revokeConfirm.title", { name: revoking?.name ?? "" })}</AlertDialogTitle>
            <AlertDialogDescription>{t("revokeConfirm.body")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{tc("cancel")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => revoking && revoke.mutate(revoking.id)}>
              {t("revoke")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
