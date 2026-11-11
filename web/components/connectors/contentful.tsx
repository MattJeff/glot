import type { ConnectorDef } from "./types";

export const contentful: ConnectorDef = {
  kind: "contentful",
  title: "Contentful",
  docs: "contentful",
  fields: [
    { name: "space_id", type: "text", placeholder: "abc123xyz789" },
    { name: "token", secret: true, placeholder: "CFPAT-…" },
    { name: "environment", type: "text", placeholder: "master", optional: true },
    { name: "api_url", type: "url", placeholder: "https://api.contentful.com", optional: true },
  ],
};
