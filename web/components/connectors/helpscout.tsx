import type { ConnectorDef } from "./types";

export const helpscout: ConnectorDef = {
  kind: "helpscout",
  title: "Help Scout Docs",
  docs: "help-scout-docs",
  fields: [
    { name: "api_key", secret: true },
    { name: "collections", type: "text", placeholder: "fr: 5f2a…, de: 61b0…" },
  ],
};
