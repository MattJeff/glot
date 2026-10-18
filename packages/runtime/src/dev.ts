// Dev-only key markers for `glot measure` / visual-qa, without instrumenting the app's code.
//
// With `GLOT_DEV_KEYS=1`, every message is prefixed with an invisible marker that encodes its key, so the
// renderer maps each element to its exact key (two "Save" buttons in two boxes get two budgets). Off
// (the default), messages are returned untouched. next-intl, in `i18n/request.ts`:
//   return { locale, messages: getGlotMessages((await import(`../messages/${locale}.json`)).default) };
//
// Marker: U+2063, the key's UTF-8 bytes as variation selectors (zero width, default-ignorable), U+2063.
// Decoded by `packages/visual-qa/src/keys.ts` (`decodeKeys`).
const SEP = "⁣";

export function encodeKey(key: string): string {
  let s = SEP;
  for (const b of new TextEncoder().encode(key)) s += String.fromCodePoint(b < 16 ? 0xfe00 + b : 0xe0100 + b - 16);
  return s + SEP;
}

const enabled = () => typeof process !== "undefined" && process.env?.GLOT_DEV_KEYS === "1";

/** Messages (nested next-intl / i18next JSON) with each string prefixed by its key's marker when `GLOT_DEV_KEYS=1`. */
export function getGlotMessages<T>(messages: T, on: boolean = enabled()): T {
  if (!on) return messages;
  const walk = (v: unknown, path: string): unknown =>
    typeof v === "string"
      ? encodeKey(path) + v
      : Array.isArray(v)
        ? v.map((x, i) => walk(x, `${path}.${i}`))
        : v && typeof v === "object"
          ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, walk(x, path ? `${path}.${k}` : k)]))
          : v;
  return walk(messages, "") as T;
}

/**
 * Wraps a translator `t(key, values)` (i18next `t`, a custom one): its output carries the key marker when
 * `GLOT_DEV_KEYS=1`. `ns` prefixes keys like next-intl's `useTranslations(ns)`.
 */
export function createGlotDevTranslator<F extends (key: string, ...rest: any[]) => any>(t: F, ns = "", on: boolean = enabled()): F {
  if (!on) return t;
  return ((key: string, ...rest: any[]) => {
    const out = t(key, ...rest);
    return typeof out === "string" ? encodeKey(ns ? `${ns}.${key}` : key) + out : out;
  }) as F;
}
