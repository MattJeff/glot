// Two-way translation (crates/server/src/two_way): what customers write back, read and answered in the team's language.
import { request } from "@/lib/api";

export type TwoWayMode = "suggest" | "one_click" | "auto";
export type TwoWaySettings = {
  team_locale: string;
  mode: TwoWayMode;
  review_below: number;
  min_detect_confidence: number;
  retention_days: number;
  reply_url: string | null;
  has_webhook_secret: boolean;
  /** Present once, in the response that created it. */
  webhook_secret?: string;
  form_enabled: boolean;
  form_store: boolean;
  form_per_day: number;
  forward_to: string | null;
  helpscout: { app_id: string | null; has_app_secret: boolean; has_webhook_secret: boolean };
  paths: { webhook: string; email: string; helpscout: string; form: string; form_script: string };
};
/** Absent = unchanged, "" clears a text field. Secrets are write-only. */
export type TwoWaySettingsInput = Partial<{
  team_locale: string;
  mode: TwoWayMode;
  review_below: number;
  reply_url: string;
  form_enabled: boolean;
  form_store: boolean;
  forward_to: string;
  generate_webhook_secret: boolean;
  helpscout_app_id: string;
  helpscout_app_secret: string;
  helpscout_webhook_secret: string;
}>;
export type TwoWayMessage = {
  id: string;
  conversation_id: string;
  direction: "in" | "out";
  original: string;
  original_locale: string | null;
  translation: string | null;
  translation_locale: string | null;
  back_translation: string | null;
  confidence: number | null;
  detect_confidence: number | null;
  flags: string[];
  pii_masked: number;
  status: "received" | "draft" | "needs_review" | "sent" | "failed";
  delivered_via: string | null;
  error: string | null;
  created_at: string;
  sent_at: string | null;
};
export type TwoWayConversationRow = {
  id: string;
  channel: "api" | "helpscout" | "form" | "email";
  external_id: string;
  customer_locale: string | null;
  customer_name: string | null;
  subject: string | null;
  last_message_at: string;
  messages: number;
  /** Replies waiting for a person (needs review or failed to send). */
  attention: number;
  preview: string | null;
};
export type TwoWayConversation = Omit<TwoWayConversationRow, "last_message_at" | "messages" | "attention" | "preview"> & {
  messages: TwoWayMessage[];
};
export type TwoWayPreview = {
  translation: string;
  back_translation: string | null;
  confidence: number;
  flags: string[];
  pii_masked: number;
  needs_review: boolean;
};
export type TwoWayStats = {
  days: number;
  total: number;
  languages: { locale: string; messages: number; share: number; served: boolean }[];
};

const enc = encodeURIComponent;
const p = (slug: string) => `/api/projects/${enc(slug)}/two-way`;

export const twoWay = {
  settings: (slug: string) => request<TwoWaySettings>("GET", `${p(slug)}/settings`),
  saveSettings: (slug: string, b: TwoWaySettingsInput) => request<TwoWaySettings>("PUT", `${p(slug)}/settings`, b),
  conversations: (slug: string) => request<{ conversations: TwoWayConversationRow[] }>("GET", `${p(slug)}/conversations`),
  conversation: (slug: string, id: string) => request<TwoWayConversation>("GET", `${p(slug)}/conversations/${enc(id)}`),
  setLanguage: (slug: string, id: string, customer_locale: string) =>
    request<TwoWayConversation>("PATCH", `${p(slug)}/conversations/${enc(id)}`, { customer_locale }),
  preview: (slug: string, conversation_id: string, text: string) =>
    request<TwoWayPreview>("POST", `${p(slug)}/preview`, { conversation_id, text }),
  reply: (slug: string, id: string, text: string, send: boolean) =>
    request<TwoWayMessage>("POST", `${p(slug)}/conversations/${enc(id)}/reply`, { text, send }),
  send: (slug: string, messageId: string) => request<TwoWayMessage>("POST", `${p(slug)}/messages/${enc(messageId)}/send`),
  stats: (slug: string) => request<TwoWayStats>("GET", `${p(slug)}/stats`),
};

export const twoWayKeys = {
  settings: (slug: string) => ["two-way", slug, "settings"] as const,
  conversations: (slug: string) => ["two-way", slug, "conversations"] as const,
  conversation: (slug: string, id: string) => ["two-way", slug, "conversation", id] as const,
  stats: (slug: string) => ["two-way", slug, "stats"] as const,
};
