// Locale routing shared by the Cloudflare Worker and the Next proxy: `/de/…` is German, anything else is the
// source unless the visitor's cookie / Accept-Language asks for an enabled locale (HTML navigations only →
// 302 to the prefixed URL, so every language has its own crawlable URL; crawlers send no Accept-Language).
import { acceptedLanguages } from "@glot/runtime/ondemand";

export interface RouteOptions {
  source: string;
  /** Locales served under `/{locale}/…`: shipped ones + those enabled on demand in glot. Required, so `/api` or `/faq` is never taken for a locale. */
  locales: readonly string[];
}
export type Route = { redirect: string } | { locale: string; path: string };

export function route(url: URL, headers: Headers, { source, locales }: RouteOptions): Route {
  const [, first = "", ...rest] = url.pathname.split("/");
  const ok = (l: string | undefined): l is string => !!l && l !== source && locales.includes(l);
  if (ok(first)) return { locale: first, path: "/" + rest.join("/") };
  // `glot_locale` cookie = explicit choice (set it to the source to opt out of redirects).
  const cookie = /(?:^|;\s*)glot_locale=([^;]+)/.exec(headers.get("cookie") ?? "")?.[1];
  const wanted = cookie ?? acceptedLanguages(headers.get("accept-language"))[0]?.toLowerCase().split("-")[0];
  if (ok(wanted) && (headers.get("accept") ?? "").includes("text/html")) return { redirect: `/${wanted}${url.pathname}${url.search}` };
  return { locale: source, path: url.pathname };
}
