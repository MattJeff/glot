import type { ConnectorDef } from "./types";

export const hubspot: ConnectorDef = {
  kind: "hubspot",
  title: "HubSpot CMS",
  docs: "hubspot-cms",
  fields: [
    { name: "token", secret: true, placeholder: "pat-na1-…" },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
