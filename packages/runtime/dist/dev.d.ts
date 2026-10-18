export declare function encodeKey(key: string): string;
/** Messages (nested next-intl / i18next JSON) with each string prefixed by its key's marker when `GLOT_DEV_KEYS=1`. */
export declare function getGlotMessages<T>(messages: T, on?: boolean): T;
/**
 * Wraps a translator `t(key, values)` (i18next `t`, a custom one): its output carries the key marker when
 * `GLOT_DEV_KEYS=1`. `ns` prefixes keys like next-intl's `useTranslations(ns)`.
 */
export declare function createGlotDevTranslator<F extends (key: string, ...rest: any[]) => any>(t: F, ns?: string, on?: boolean): F;
