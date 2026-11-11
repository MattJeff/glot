import type { ConnectorDef } from "./types";

export const shopware: ConnectorDef = {
  kind: "shopware",
  title: "Shopware 6",
  docs: "shopware",
  fields: [
    { name: "url", type: "url", placeholder: "https://shop.example.com" },
    { name: "client_id", type: "text", placeholder: "SWIA…" },
    { name: "client_secret", secret: true },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
