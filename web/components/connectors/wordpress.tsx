import type { ConnectorDef } from "./types";

export const wordpress: ConnectorDef = {
  kind: "wordpress",
  title: "WordPress",
  docs: "wordpress",
  fields: [
    { name: "url", type: "url", placeholder: "https://example.com" },
    { name: "username", type: "text" },
    { name: "app_password", secret: true, placeholder: "xxxx xxxx xxxx xxxx xxxx xxxx" },
  ],
};
