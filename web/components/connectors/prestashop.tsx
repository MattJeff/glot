import type { ConnectorDef } from "./types";

export const prestashop: ConnectorDef = {
  kind: "prestashop",
  title: "PrestaShop",
  docs: "prestashop",
  fields: [
    { name: "url", type: "url", placeholder: "https://shop.example.com" },
    { name: "api_key", secret: true, placeholder: "ZR92FNY5UFRERNI3O9Z5QDHWKTP3YIIT" },
  ],
};
