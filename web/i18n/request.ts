import { readFile } from "node:fs/promises";
import path from "node:path";
import { hasLocale } from "next-intl";
import { getRequestConfig } from "next-intl/server";
import en from "../messages/en.json";
import { routing } from "./routing";

type Json = { [k: string]: string | Json };

function deepMerge(base: Json, over: Json): Json {
  const out: Json = { ...base };
  for (const [k, v] of Object.entries(over)) {
    const b = out[k];
    out[k] = typeof v === "object" && typeof b === "object" ? deepMerge(b, v) : v;
  }
  return out;
}

// en ⊕ locale: untranslated keys fall back to the English source until `glot translate` runs.
async function loadMessages(locale: string): Promise<Json> {
  if (locale === routing.defaultLocale) return en;
  try {
    const file = path.join(process.cwd(), "messages", `${locale}.json`);
    return deepMerge(en, JSON.parse(await readFile(file, "utf8")));
  } catch {
    return en;
  }
}

export default getRequestConfig(async ({ requestLocale }) => {
  const requested = await requestLocale;
  const locale = hasLocale(routing.locales, requested) ? requested : routing.defaultLocale;
  return { locale, messages: await loadMessages(locale) };
});
