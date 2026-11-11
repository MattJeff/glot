import type { ConnectorDef } from "./types";

export const ghost: ConnectorDef = {
  kind: "ghost",
  title: "Ghost",
  docs: "ghost",
  fields: [
    { name: "url", type: "url", placeholder: "https://blog.example.com" },
    { name: "admin_key", secret: true, placeholder: "6489…:0123…" },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
