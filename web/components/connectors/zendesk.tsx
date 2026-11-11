import type { ConnectorDef } from "./types";

export const zendesk: ConnectorDef = {
  kind: "zendesk",
  title: "Zendesk Guide",
  docs: "zendesk-guide",
  fields: [
    { name: "subdomain", type: "text", placeholder: "acme" },
    { name: "email", type: "text", placeholder: "admin@example.com" },
    { name: "token", secret: true },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
