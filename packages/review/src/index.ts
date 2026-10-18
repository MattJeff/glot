// @glot/review — live review of translations on the real site. This entry is tiny: it only decides whether
// the visitor is a reviewer (review link `?glot_review=<token>`, a stored session, or `token`) and, if so,
// lazy-loads the widget. Normal visitors download nothing more and nothing runs.
import type { ReviewOptions } from "./types";
export type { ReviewOptions, Labels } from "./types";

const SS = "glot-review";

/** Review token for this tab, or "" (normal visitor). Strips `glot_review` from the address bar. */
export function reviewToken(explicit?: string): string {
  if (typeof location === "undefined") return "";
  const u = new URL(location.href);
  const fromUrl = u.searchParams.get("glot_review");
  let t = explicit || fromUrl || "";
  try {
    if (fromUrl) sessionStorage.setItem(SS, fromUrl);
    t ||= sessionStorage.getItem(SS) || "";
  } catch {}
  if (fromUrl) { u.searchParams.delete("glot_review"); history.replaceState(history.state, "", u); }
  return t;
}

/** Starts review mode if the visitor is a reviewer. Resolves to true when the widget was loaded. */
export async function init(opts: ReviewOptions): Promise<boolean> {
  const token = reviewToken(opts.token);
  if (!token) return false;
  (await import("./app")).start({ ...opts, token });
  return true;
}

export const glotReview = { init };
