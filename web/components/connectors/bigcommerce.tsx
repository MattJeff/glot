import type { ConnectorDef } from "./types";

export const bigcommerce: ConnectorDef = {
  kind: "bigcommerce",
  title: "BigCommerce",
  docs: "bigcommerce",
  fields: [
    { name: "store_hash", type: "text", placeholder: "abc123xyz" },
    { name: "token", secret: true },
    { name: "channel_id", type: "text", placeholder: "1", optional: true },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
