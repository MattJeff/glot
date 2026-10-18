import { getRequestConfig } from "next-intl/server";
import { headers } from "next/headers";
import { createOnDemandLoader, acceptedLanguages } from "@glot/runtime/ondemand";
import { withExperiments } from "@glot/runtime/experiments";
import en from "../messages/en.json";

// The app ships English only. With GLOT_SERVER set, any other browser language is asked to glot on demand:
// the first visit renders English (+ on-device fallback in the browser) while glot translates; every visit
// after that is server-rendered in that language (real HTML, indexable).
const ondemand = process.env.GLOT_SERVER
  ? createOnDemandLoader({ server: process.env.GLOT_SERVER, project: process.env.GLOT_PROJECT ?? "runtime-next", source: en })
  : null;

export default getRequestConfig(async () => {
  // `x-glot-locale`: set by `glotProxy` (@glot/edge/next) for `/de/…` URLs; else the browser's first language.
  // ponytail: language subtag only (de-AT → de); pass the full tag if you ship regional variants.
  const h = await headers();
  const lang = h.get("x-glot-locale") ?? acceptedLanguages(h.get("accept-language"))[0]?.toLowerCase().split("-")[0];
  const od = ondemand && lang && lang !== "en" ? await ondemand(lang) : null;
  const locale = od ? lang! : "en";
  // Order: catalog (shipped or on demand) → experiment overlay drawn by the edge (`x-glot-xp`, served locale only)
  // → in the browser, the on-device fallback translates whatever the server rendered.
  const messages = withExperiments(od ?? en, { header: h.get("x-glot-xp"), locale });
  return { locale, timeZone: "UTC", messages };
});
