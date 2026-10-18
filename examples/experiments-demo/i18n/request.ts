import { getRequestConfig } from "next-intl/server";
import { headers } from "next/headers";
import { withExperiments } from "@glot/runtime/experiments";
import en from "../messages/en.json";
import fr from "../messages/fr.json";
import de from "../messages/de.json";

const catalogs = { en, fr, de } as Record<string, typeof en>;

// `x-glot-locale` and `x-glot-xp` are set by glotProxy (proxy.ts): the HTML is rendered with the arm the edge drew.
export default getRequestConfig(async () => {
  const h = await headers();
  const locale = h.get("x-glot-locale") ?? "en";
  const messages = catalogs[locale] ?? en;
  return { locale, timeZone: "UTC", messages: withExperiments(messages, { header: h.get("x-glot-xp"), locale }) };
});
