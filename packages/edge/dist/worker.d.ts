export interface Env {
    GLOT_SERVER: string;
    GLOT_PROJECT: string;
    /** Comma-separated locales served under `/{locale}/` (glot's on-demand whitelist). */
    LOCALES: string;
    SOURCE_LOCALE?: string;
    /** Project public key (`pk_…`): enables segmented copy experiments. */
    GLOT_KEY?: string;
}
declare const _default: {
    fetch(req: Request, env: Env, ctx?: {
        waitUntil(p: Promise<unknown>): void;
    }): Promise<Response>;
};
export default _default;
