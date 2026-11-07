// "Is this HTML an empty SPA shell?" — the `rendering: auto` heuristic. Same rule in Rust (glot_core::edge::render).
// Visible text of <body> (scripts, styles, templates, noscript and comments removed, tags stripped, spaces folded)
// under MIN_TEXT characters, or no content element at all (h1–h3, p, li, main text) → skeleton → use a snapshot.
// ponytail: regex strip, not a parser — enough to tell "<div id=root></div>" from a real page; a page that is
// legitimately short (a 404, a login form) is rendered too, which is harmless (snapshot ≈ origin).
export const MIN_TEXT = 200;

const content = (html: string) =>
  (/<body[^>]*>([\s\S]*)<\/body>/i.exec(html)?.[1] ?? html)
    .replace(/<!--[\s\S]*?-->/g, " ")
    .replace(/<(script|style|template|noscript|svg)\b[\s\S]*?<\/\1>/gi, " ");

export function visibleText(html: string): string {
  return content(html).replace(/<[^>]+>/g, " ").replace(/&[a-z#0-9]+;/gi, " ").replace(/\s+/g, " ").trim();
}

export function isSkeleton(html: string): boolean {
  const c = content(html);
  return visibleText(c).length < MIN_TEXT || !/<(h1|h2|h3|p|li)\b/i.test(c);
}
