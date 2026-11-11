import type { ConnectorDef } from "./types";

export const woocommerce: ConnectorDef = {
  kind: "woocommerce",
  title: "WooCommerce",
  docs: "woocommerce",
  fields: [
    { name: "url", type: "url", placeholder: "https://shop.example.com" },
    { name: "consumer_key", type: "text", placeholder: "ck_…" },
    { name: "consumer_secret", secret: true, placeholder: "cs_…" },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
