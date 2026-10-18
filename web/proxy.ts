import createMiddleware from "next-intl/middleware";
import { NextResponse, type NextRequest } from "next/server";
import { routing } from "./i18n/routing";

const intl = createMiddleware(routing);
const APP_PATH = new RegExp(`^(/(${routing.locales.join("|")}))?/app(/|$)`);

export default function proxy(req: NextRequest) {
  // Cheap gate: no session cookie => login. A stale cookie is caught client-side on the first 401 from /api/me.
  if (APP_PATH.test(req.nextUrl.pathname) && !req.cookies.has("glot_session")) {
    const url = req.nextUrl.clone();
    const prefix = req.nextUrl.pathname.match(APP_PATH)?.[1] ?? "";
    url.pathname = `${prefix}/login`;
    url.search = `?next=${encodeURIComponent(req.nextUrl.pathname)}`;
    return NextResponse.redirect(url);
  }
  return intl(req);
}

export const config = {
  matcher: "/((?!api|cdn|_next|_vercel|.*\\..*).*)",
};
