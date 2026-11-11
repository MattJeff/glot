import type { ConnectorDef } from "./types";

export const shopify: ConnectorDef = {
  kind: "shopify",
  title: "Shopify",
  docs: "shopify",
  fields: [
    { name: "shop", type: "text", placeholder: "my-store.myshopify.com" },
    { name: "token", secret: true, placeholder: "shpat_…" },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
