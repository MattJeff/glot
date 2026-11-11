import type { ConnectorDef } from "./types";

export const sanity: ConnectorDef = {
  kind: "sanity",
  title: "Sanity",
  docs: "sanity",
  fields: [
    { name: "project_id", type: "text", placeholder: "abc123de" },
    { name: "dataset", type: "text", placeholder: "production" },
    { name: "token", secret: true, placeholder: "sk…" },
    { name: "source_locale", type: "text", placeholder: "en", optional: true },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
