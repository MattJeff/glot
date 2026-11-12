import type { ConnectorDef } from "./types";

export const webflow: ConnectorDef = {
  kind: "webflow",
  title: "Webflow",
  docs: "webflow",
  oneClick: {},
  fields: [
    { name: "token", secret: true },
    { name: "site_id", type: "text", optional: true },
    { name: "webhook_secret", secret: true, optional: true },
  ],
};
