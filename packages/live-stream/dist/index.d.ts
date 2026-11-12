export type LiveEvent = 
/** One final piece of output. `via`: raw (structure/code), llm, cache, tm (translation memory), source (not translated, see `reason`). */
{
    type: "delta";
    text: string;
    n: number;
    via: "raw" | "llm" | "cache" | "tm" | "source";
    reason?: string;
}
/** The helper gave up on glot for the rest of this stream and passes the source through. */
 | {
    type: "fallback";
    reason: string;
};
export type LiveOptions = {
    /** Project slug. */
    project: string;
    /** Target locale (`es`, `pt-BR`). */
    to: string;
    /** Source locale; omit to let the model detect it. */
    from?: string;
    /** Server side: a glot API token (never ship it to a browser). */
    token?: string;
    /** Browser: the project's public key (`pk_…`); needs "Browser access" enabled on the project. */
    key?: string;
    /** glot API origin. Default: https://api.glotlang.com */
    endpoint?: string;
    /** Full URL of the translate route, when it is proxied (wins over `endpoint`). */
    url?: string;
    /** Same id for every turn of a conversation: repeated sentences and wording stay consistent. */
    conversation?: string;
    signal?: AbortSignal;
    onEvent?: (e: LiveEvent) => void;
    fetch?: typeof fetch;
};
type Source = AsyncIterable<string | Uint8Array> | ReadableStream<string | Uint8Array>;
/** The translated stream as an async generator of text. */
export declare function translateIterable(source: Source, opts: LiveOptions): AsyncGenerator<string, void, undefined>;
/** The translated stream as a `ReadableStream<string>` (pipe it through `TextEncoderStream` for a `Response`). */
export declare function translateStream(source: Source, opts: LiveOptions): ReadableStream<string>;
export {};
