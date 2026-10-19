import { NextResponse, type NextRequest } from "next/server";
import { routing } from "./routing";

const COOKIE = "NEXT_LOCALE";

/** Locale from the URL prefix (`/fr/...`) or the NEXT_LOCALE cookie, passed on to i18n/request.ts. */
export function i18nProxy(req: NextRequest) {
  const { locales, defaultLocale, localePrefix } = routing;
  const path = req.nextUrl.pathname;
  const first = path.split("/")[1];
  const prefixed = locales.includes(first);
  const saved = req.cookies.get(COOKIE)?.value;
  const locale = prefixed ? first : saved && locales.includes(saved) ? saved : defaultLocale;
  const rest = (prefixed ? path.slice(first.length + 1) : path) || "/";
  const url = req.nextUrl.clone();
  let res: NextResponse;
  if (localePrefix === "prefix" && (prefixed ? locale === defaultLocale : locale !== defaultLocale)) {
    url.pathname = locale === defaultLocale ? rest : `/${locale}${rest === "/" ? "" : rest}`;
    res = NextResponse.redirect(url);
  } else {
    url.pathname = `/${locale}${rest === "/" ? "" : rest}`;
    res = prefixed ? NextResponse.next() : NextResponse.rewrite(url);
  }
  if (prefixed && saved !== locale) {
    res.cookies.set(COOKIE, locale, { path: "/", maxAge: 60 * 60 * 24 * 365, sameSite: "lax" });
  }
  return res;
}
