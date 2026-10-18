import { readFileSync } from "node:fs";
import { request, type APIRequestContext } from "@playwright/test";

export const API = process.env.API_URL ?? "http://localhost:8832";
export const APP = process.env.APP_URL ?? "http://localhost:3520";
export const SITE = process.env.SITE_URL ?? "http://localhost:3530";
export const SLUG = process.env.GLOT_PROJECT ?? "relay";
export const OWNER = { email: "owner@review-demo.test", password: "review-demo-password", name: "Mathis" };

export type State = { comment: string; read: string; edit: string; member: string; apiToken: string };
export const state = (): State => JSON.parse(readFileSync(new URL("./.state.json", import.meta.url), "utf8"));

/** API client authenticated with a widget token (glr_…), sending the site's Origin like the widget does. */
export const widget = (token: string) =>
  request.newContext({ baseURL: `${API}/api/review/w/`, extraHTTPHeaders: { authorization: `Bearer ${token}`, origin: SITE } });
/** API client authenticated as the project owner (API token, like the CLI/MCP). */
export const member = (s = state()) =>
  request.newContext({ baseURL: `${API}/api/projects/${SLUG}/review/`, extraHTTPHeaders: { authorization: `Bearer ${s.apiToken}` } });

export const anchor = (over: Record<string, unknown> = {}) => ({
  selector: "#hero-title", text: "Rechnungen, die pünktlich bezahlt werden", url: `${SITE}/de?x=1#top`, locale: "de",
  viewport: { w: 1280, h: 860 }, rect: { x: 200, y: 120, w: 880, h: 60 }, point: { x: 0.5, y: 0.5 }, ...over,
});

export async function json<T = any>(r: Awaited<ReturnType<APIRequestContext["get"]>>): Promise<T> {
  if (!r.ok()) throw new Error(`${r.status()} ${r.url()}: ${await r.text()}`);
  return r.status() === 204 ? (undefined as T) : r.json();
}
