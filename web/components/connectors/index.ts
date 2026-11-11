// Every CMS connector of the Integrations page. Adding one: `<kind>.tsx` exporting a ConnectorDef, one line here,
// its messages under `connectors.<kind>` in the 6 catalogs (and the server side in crates/server/src/connectors).
import { bigcommerce } from "./bigcommerce";
import { contentful } from "./contentful";
import { craft } from "./craft";
import { drupal } from "./drupal";
import { ghost } from "./ghost";
import { helpscout } from "./helpscout";
import { hubspot } from "./hubspot";
import { magento } from "./magento";
import { prestashop } from "./prestashop";
import { sanity } from "./sanity";
import { shopify } from "./shopify";
import { shopware } from "./shopware";
import { strapi } from "./strapi";
import type { ConnectorDef } from "./types";
import { webflow } from "./webflow";
import { woocommerce } from "./woocommerce";
import { wordpress } from "./wordpress";
import { zendesk } from "./zendesk";

export const CONNECTORS: ConnectorDef[] = [shopify, wordpress, webflow, contentful, sanity, strapi, woocommerce, bigcommerce, prestashop, magento, shopware, hubspot, drupal, craft, ghost, zendesk, helpscout];
export type { ConnectorDef, ConnectorField } from "./types";
