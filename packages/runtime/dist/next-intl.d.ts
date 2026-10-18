import { type ReactNode } from "react";
import { type AutoOptions } from "./index.js";
export interface GlotFallbackProps extends Omit<AutoOptions, "locale" | "onMessages"> {
    /** Locales the app ships (e.g. `routing.locales`). */
    shipped: readonly string[];
    /** Force a locale (default: `navigator.languages[0]` when not shipped). */
    locale?: string;
    children: ReactNode;
}
type Values = Record<string, string | number | Date>;
export declare function GlotFallbackProvider({ children, shipped, locale, ...o }: GlotFallbackProps): string | number | bigint | boolean | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | Promise<string | number | bigint | boolean | import("react").ReactPortal | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | null | undefined> | import("react").FunctionComponentElement<import("react").ProviderProps<((key: string, values?: Values) => string | null) | null>> | null | undefined;
/**
 * Server-rendered text that follows the on-device translation. `children` is what the server rendered
 * (shown as-is until a fallback catalog exists, so hydration always matches); `k` is the full dotted key.
 */
export declare function T({ k, values, children }: {
    k: string;
    values?: Values;
    children?: ReactNode;
}): ReactNode;
/** Variant text served to this visitor for `key` (full dotted key), or undefined: `useGlotVariant(k) ?? t(k)`. */
export declare function useGlotVariant(key: string): string | undefined;
/**
 * Inside your NextIntlClientProvider: every `t(key)` in Client Components below renders this visitor's copy
 * variant (ICU placeholders still work), so experiments need no code change at the call sites.
 */
export declare function GlotVariants({ children }: {
    children: ReactNode;
}): string | number | bigint | boolean | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | Promise<string | number | bigint | boolean | import("react").ReactPortal | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | null | undefined> | import("react").FunctionComponentElement<Omit<import("next-intl").IntlConfig & {
    children: ReactNode;
}, "locale"> & {
    locale?: import("next-intl").Locale;
}> | null | undefined;
export {};
