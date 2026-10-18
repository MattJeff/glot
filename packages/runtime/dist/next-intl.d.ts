import { type ReactNode } from "react";
import { type AutoOptions } from "./index.js";
export interface GlotFallbackProps extends Omit<AutoOptions, "locale" | "onMessages"> {
    /** Locales the app ships (e.g. `routing.locales`). */
    shipped: readonly string[];
    /** Force a locale (default: `navigator.languages[0]` when not shipped). */
    locale?: string;
    children: ReactNode;
}
export declare function GlotFallbackProvider({ children, shipped, locale, ...o }: GlotFallbackProps): string | number | bigint | boolean | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | Promise<string | number | bigint | boolean | import("react").ReactPortal | import("react").ReactElement<unknown, string | import("react").JSXElementConstructor<any>> | Iterable<ReactNode> | null | undefined> | import("react").FunctionComponentElement<Omit<import("next-intl").IntlConfig & {
    children: ReactNode;
}, "locale"> & {
    locale?: import("next-intl").Locale;
}> | null | undefined;
