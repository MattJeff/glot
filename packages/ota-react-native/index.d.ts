/** Anything with AsyncStorage's getItem / setItem (e.g. `@react-native-async-storage/async-storage`). */
export interface GlotStorage {
  getItem(key: string): Promise<string | null> | string | null;
  setItem(key: string, value: string): Promise<void> | void;
}

export interface GlotOtaOptions {
  /** Project slug (`/cdn/{project}/…`); its CDN must be public. */
  project: string;
  /** Your glot server, e.g. `https://api.example.com`. */
  cdn: string;
  /** Where the last downloaded catalogs are kept (AsyncStorage). Without it, nothing survives a restart. */
  storage?: GlotStorage;
  /** The JSON shipped in the app, by language then namespace (or a loader): the fallback for every key. */
  bundled?:
    | Record<string, Record<string, Record<string, unknown>>>
    | ((language: string, namespace: string) => Record<string, unknown> | Promise<Record<string, unknown>>);
  /** Called after newer strings were added to i18next (they are live with `react: { bindI18nStore: "added" }`). */
  onUpdate?: (language: string) => void;
  /** Defaults to the global `fetch`. */
  fetch?: (url: string, init: { headers: Record<string, string>; credentials: "omit" }) => Promise<{
    status: number;
    headers: { get(name: string): string | null };
    text(): Promise<string>;
  }>;
}

/** First ready language matching `wanted` (exact, then same language), else `wanted`'s language. */
export function pick(wanted: string, available: string[]): string;

export class GlotOtaBackend {
  static type: "backend";
  type: "backend";
  constructor(services?: unknown, options?: GlotOtaOptions);
  init(services: unknown, options?: GlotOtaOptions): void;
  read(language: string, namespace: string, callback: (err: unknown, data?: unknown) => void): void;
  /** Languages glot has ready, source first: offer them in a picker, embedded in the app or not. */
  locales(): Promise<string[]>;
}

export default GlotOtaBackend;
