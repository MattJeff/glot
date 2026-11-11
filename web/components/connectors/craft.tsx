import type { ConnectorDef } from "./types";

export const craft: ConnectorDef = {
  kind: "craft",
  title: "Craft CMS",
  docs: "craft-cms",
  fields: [
    { name: "url", type: "url", placeholder: "https://www.example.com/api" },
    { name: "token", secret: true },
    { name: "sites", type: "text", placeholder: "fr: french, de: german", optional: true },
  ],
};
