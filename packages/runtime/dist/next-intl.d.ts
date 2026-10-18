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
export {};
