// i18next backend plugin: serves `source` for the source language and the on-device translation for any
// other language. Chain it after your normal backend (i18next-chained-backend) to only cover unshipped ones.
// After a click-to-download the bundle is added later: use `react: { bindI18nStore: "added" }` to re-render.
import { fallbackWithBadge, type AutoOptions } from "./index.js";

export interface GlotBackendOptions extends Omit<AutoOptions, "locale" | "source" | "onMessages" | "syntax"> {
  /** Source catalog per namespace. */
  source: Record<string, unknown> | ((ns: string) => Record<string, unknown> | Promise<Record<string, unknown>>);
}

export class GlotFallbackBackend {
  static type = "backend" as const;
  type = "backend" as const;
  private services: any;
  private o!: GlotBackendOptions;

  constructor(services?: unknown, options?: GlotBackendOptions) {
    if (services) this.init(services, options!);
  }

  init(services: unknown, options: GlotBackendOptions) {
    this.services = services;
    this.o = options;
  }

  read(language: string, namespace: string, cb: (err: unknown, data?: unknown) => void) {
    const { source, ...o } = this.o;
    Promise.resolve(typeof source === "function" ? source(namespace) : source)
      .then(async (src) => {
        let first = true;
        const r = await fallbackWithBadge({
          ...o,
          syntax: "i18next",
          source: src as any,
          locale: language,
          onMessages: (m) => {
            if (!first) this.services?.resourceStore?.addResourceBundle(language, namespace, m, true, true);
          },
        });
        first = false;
        cb(null, r.messages);
      })
      .catch((e) => cb(e));
  }
}
