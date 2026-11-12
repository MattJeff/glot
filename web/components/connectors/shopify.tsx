import type { ConnectorDef } from "./types";

export const shopify: ConnectorDef = {
  kind: "shopify",
  title: "Shopify",
  docs: "shopify",
  oneClick: { tenant: { placeholder: "my-store.myshopify.com" } },
  // Manual fallback: the store's own Dev Dashboard app (client ID + secret, tokens renewed by glot),
  // or a legacy custom app's static token.
  fields: [
    { name: "shop", type: "text", placeholder: "my-store.myshopify.com" },
    { name: "client_id", type: "text", optional: true },
    { name: "client_secret", secret: true, optional: true },
    { name: "token", secret: true, placeholder: "shpat_…", optional: true },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
