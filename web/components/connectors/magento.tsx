import type { ConnectorDef } from "./types";

export const magento: ConnectorDef = {
  kind: "magento",
  title: "Magento / Adobe Commerce",
  docs: "magento-adobe-commerce",
  fields: [
    { name: "url", type: "url", placeholder: "https://shop.example.com" },
    { name: "token", secret: true },
    { name: "token_secret", secret: true, optional: true },
    { name: "consumer_key", secret: true, optional: true },
    { name: "consumer_secret", secret: true, optional: true },
  ],
};
