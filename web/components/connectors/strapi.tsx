import type { ConnectorDef } from "./types";

export const strapi: ConnectorDef = {
  kind: "strapi",
  title: "Strapi",
  docs: "strapi",
  fields: [
    { name: "url", type: "url", placeholder: "https://cms.example.com" },
    { name: "token", secret: true },
  ],
};
