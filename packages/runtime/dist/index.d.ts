import { showBadge, type BadgeOptions } from "./badge";
import { type Glossary, type Syntax } from "./mask";
export { mask, parseIcu, i18nextPluralKeys, type Glossary, type Term, type Syntax } from "./mask";
export { showBadge, type BadgeOptions };
export declare const VERSION = "0.1.0";
type Json = {
    [k: string]: string | Json | unknown;
};
export type Status = "translated" | "source" | "unsupported" | "unavailable" | "needs-activation" | "validator-unavailable" | "error";
export interface FallbackResult {
    messages: Json;
    status: Status;
    locale: string;
    /** Keys (dotted) that failed masking/translation/validation and stayed in the source language. */
    fellBack: string[];
    translated: number;
    cached: number;
}
export interface FallbackOptions {
    /** Source catalog (nested JSON, e.g. `messages/en.json`). */
    source: Json;
    locale: string;
    sourceLocale?: string;
    glossary?: Glossary;
    syntax?: Syntax;
    /** Model download progress, 0..1. */
    onProgress?: (loaded: number) => void;
    /** Where `glot.wasm` lives (default: next to this module). Or the bytes. */
    wasm?: string | URL | BufferSource;
    /** IndexedDB cache (default on when IndexedDB exists). */
    cache?: boolean;
    /** Cache key part; the Translator API exposes no model version, so default = browser build + runtime version. */
    modelVersion?: string;
    /** Max characters per translate() call (segments joined by newlines). */
    batchChars?: number;
}
interface TranslatorLike {
    translate(text: string): Promise<string>;
}
export declare const isSupported: () => boolean;
/** Availability of the pair: "unavailable" | "downloadable" | "downloading" | "available" | "unsupported". */
export declare function availability(sourceLanguage: string, targetLanguage: string): Promise<string>;
/** Create (and download if needed) the translator. Needs a user activation when the model isn't downloaded yet. */
export declare function prepareTranslator(sourceLanguage: string, targetLanguage: string, onProgress?: (loaded: number) => void): Promise<TranslatorLike>;
export interface Issue {
    code: string;
    severity: "error" | "warning";
    message: string;
}
export type Validate = (input: {
    source: string;
    target: string;
    locale: string;
    syntax?: Syntax;
    glossary?: unknown[];
}) => Issue[];
/** Load glot's validator (crates/wasm). Cached after the first call. */
export declare function loadValidator(src?: string | URL | BufferSource): Promise<Validate>;
/**
 * Translate `source` into `locale` on the device. Never throws: on any problem the result is the source
 * catalog with a `status` explaining why. Safe to call on every page load (IndexedDB cache).
 */
export declare function createFallbackMessages(opts: FallbackOptions): Promise<FallbackResult>;
/** First browser language that the app doesn't ship (and isn't the source), or null. */
export declare function wantedLocale(shipped: readonly string[], languages?: readonly string[]): string | null;
export interface AutoOptions extends Omit<FallbackOptions, "onProgress"> {
    /** `false` hides the badge (and the click-to-download prompt). */
    badge?: false | Pick<BadgeOptions, "label" | "improveLabel" | "improveUrl" | "activateLabel">;
    /** Called with the translated catalog — right away, or later once the user allowed the model download. */
    onMessages?: (messages: Json, result: FallbackResult) => void;
}
/**
 * createFallbackMessages + badge: shows "translated on your device · Improve", the download percentage, and
 * when the model isn't downloaded yet a "Translate this page on your device" button (the API needs a click).
 * The badge texts are translated along with the catalog.
 */
export declare function fallbackWithBadge(o: AutoOptions): Promise<FallbackResult>;
