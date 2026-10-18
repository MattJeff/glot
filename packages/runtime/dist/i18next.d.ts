import { type AutoOptions } from "./index.js";
export interface GlotBackendOptions extends Omit<AutoOptions, "locale" | "source" | "onMessages" | "syntax"> {
    /** Source catalog per namespace. */
    source: Record<string, unknown> | ((ns: string) => Record<string, unknown> | Promise<Record<string, unknown>>);
}
export declare class GlotFallbackBackend {
    static type: "backend";
    type: "backend";
    private services;
    private o;
    constructor(services?: unknown, options?: GlotBackendOptions);
    init(services: unknown, options: GlotBackendOptions): void;
    read(language: string, namespace: string, cb: (err: unknown, data?: unknown) => void): void;
}
