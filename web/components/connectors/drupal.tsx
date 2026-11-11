import type { ConnectorDef } from "./types";

export const drupal: ConnectorDef = {
  kind: "drupal",
  title: "Drupal",
  docs: "drupal",
  fields: [
    { name: "url", type: "url", placeholder: "https://www.example.com" },
    { name: "username", type: "text", optional: true },
    { name: "password", secret: true, optional: true },
    { name: "token", secret: true, optional: true },
  ],
};
