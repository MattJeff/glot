import { writeFileSync } from "node:fs";
import { request } from "@playwright/test";
import de from "../messages/de.json" with { type: "json" };
import en from "../messages/en.json" with { type: "json" };
import fr from "../messages/fr.json" with { type: "json" };
import { API, OWNER, SITE, SLUG, json, type State } from "./env";

// Owner account + project `relay` (en → fr, de) with the site's catalogs, the site origin allowed for the
// widget, three review links (read / comment / edit), a member widget session and an API token (CLI/MCP).
// Idempotent: reuses the account/project on a second run against the same database.
export default async function setup() {
  const api = await request.newContext({ baseURL: API });
  const up = await api.post("/api/auth/signup", { data: OWNER });
  if (!up.ok()) await json(await api.post("/api/auth/login", { data: { email: OWNER.email, password: OWNER.password } }));
  const p = await api.post("/api/projects", { data: { name: "Relay", slug: SLUG, source_locale: "en", target_locales: ["fr", "de"] } });
  if (!p.ok() && p.status() !== 409) throw new Error(`project: ${p.status()} ${await p.text()}`);
  await json(await api.post(`/api/projects/${SLUG}/sync/push`, { data: { catalogs: { en, fr, de }, glossary: [], context: {} } }));
  const R = `/api/projects/${SLUG}/review`;
  await json(await api.put(`${R}/settings`, { data: { origins: [SITE] } }));
  const link = async (access: string) => (await json(await api.post(`${R}/links`, { data: { label: `e2e ${access}`, access } }))).token as string;
  const s: State = {
    read: await link("read"),
    comment: await link("comment"),
    edit: await link("edit"),
    member: (await json(await api.post(`${R}/session`))).token,
    apiToken: (await json(await api.post("/api/tokens", { data: { name: "review-demo e2e" } }))).token,
  };
  writeFileSync(new URL("./.state.json", import.meta.url), JSON.stringify(s, null, 2));
  await api.dispose();
}
