export interface Env {
    GLOT_SERVER: string;
    GLOT_PROJECT: string;
    /** Comma-separated locales served under `/{locale}/` (glot's on-demand whitelist). */
    LOCALES: string;
    SOURCE_LOCALE?: string;
}
declare const _default: {
    fetch(req: Request, env: Env): Promise<Response>;
};
export default _default;
