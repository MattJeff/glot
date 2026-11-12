// Translate a text stream while it streams (glot live stream).
// `translateStream(source, opts)` takes what an LLM SDK gives you (a ReadableStream or an async iterable of text
// or bytes) and returns the same kind of stream in another language. Works in Node, edge runtimes and browsers:
// plain `fetch`, no WebSocket.
//
// What comes out is final: text is held back until a sentence (or a line) is complete, then emitted translated,
// in order, and never changed afterwards. Code blocks, inline code, links and numbers come out untouched.
// Fail-open: if glot cannot be reached or refuses, the source text passes through unchanged and `onEvent`
// receives `{ type: "fallback" }`; your user always gets the whole message.

export type LiveEvent =
  /** One final piece of output. `via`: raw (structure/code), llm, cache, tm (translation memory), source (not translated, see `reason`). */
  | { type: "delta"; text: string; n: number; via: "raw" | "llm" | "cache" | "tm" | "source"; reason?: string }
  /** The helper gave up on glot for the rest of this stream and passes the source through. */
  | { type: "fallback"; reason: string };

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

async function* chunks(source: Source): AsyncGenerator<string> {
  const dec = new TextDecoder();
  const text = (c: string | Uint8Array) => (typeof c === "string" ? c : dec.decode(c, { stream: true }));
  if (Symbol.asyncIterator in source) {
    for await (const c of source) yield text(c);
    return;
  }
  const reader = (source as ReadableStream<string | Uint8Array>).getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      yield text(value);
    }
  } finally {
    void reader.cancel().catch(() => {});
  }
}

/** Server-Sent Events of a fetch response: `[event, data]` pairs. */
async function* sse(res: Response): AsyncGenerator<[string, string]> {
  const reader = res.body!.getReader();
  const dec = new TextDecoder();
  let buf = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buf += dec.decode(value, { stream: true });
    for (let end; (end = buf.indexOf("\n\n")) >= 0; buf = buf.slice(end + 2)) {
      let event = "message";
      const data: string[] = [];
      for (const line of buf.slice(0, end).split("\n")) {
        if (line.startsWith("event:")) event = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
      }
      if (data.length) yield [event, data.join("\n")];
    }
  }
}

/** Above this many pending characters the source is no longer read (back-pressure on the producer). */
const HIGH_WATER = 16_000;

/** The translated stream as an async generator of text. */
export async function* translateIterable(source: Source, opts: LiveOptions): AsyncGenerator<string, void, undefined> {
  const doFetch = opts.fetch ?? fetch;
  const url =
    opts.url ??
    `${opts.endpoint ?? "https://api.glotlang.com"}${
      opts.token ? `/api/projects/${opts.project}/live-stream/translate` : `/live/v1/${opts.project}/translate`
    }`;
  const abort = new AbortController();
  const onAbort = () => abort.abort();
  opts.signal?.addEventListener("abort", onAbort);

  // The source is read in the background into `pending`; the loop below sends what is pending, one request at
  // a time, and drops what the server consumed.
  let pending = "";
  let ended = false;
  let failure: unknown;
  let wake = () => {};
  const sleep = () => new Promise<void>((r) => (wake = r));
  const pump = (async () => {
    try {
      for await (const c of chunks(source)) {
        pending += c;
        wake();
        while (pending.length > HIGH_WATER && !abort.signal.aborted) await sleep();
        if (abort.signal.aborted) return;
      }
    } catch (e) {
      failure = e;
    } finally {
      ended = true;
      wake();
    }
  })();

  let state: { fence: string | null; mid: boolean } = { fence: null, mid: false };
  let passthrough = false;
  let sent = 0; // length of `pending` at the last request: nothing new = nothing to ask
  try {
    for (;;) {
      // Ask only when a sentence or a line may have ended (or a lot is waiting, or the stream is over).
      while (!ended && !(pending.length > sent && (/[\n.!?…。！？]/.test(pending.slice(sent)) || pending.length - sent > 400))) {
        await sleep();
      }
      if (abort.signal.aborted) return;
      const text = pending;
      const final = ended;
      if (text === "" && final) break;
      if (passthrough) {
        pending = pending.slice(text.length);
        wake();
        yield text;
        continue;
      }
      let consumed = 0; // UTF-16 units of `text` already answered by deltas
      try {
        const res = await doFetch(url, {
          method: "POST",
          // text/plain from browsers: no CORS preflight (the body is JSON either way).
          headers: opts.token
            ? { "content-type": "application/json", authorization: `Bearer ${opts.token}` }
            : { "content-type": opts.url ? "application/json" : "text/plain" },
          body: JSON.stringify({
            to: opts.to,
            from: opts.from,
            text,
            final,
            conversation: opts.conversation,
            fence: state.fence,
            mid: state.mid,
            key: opts.key,
          }),
          signal: abort.signal,
        });
        if (!res.ok || !res.body) {
          const err = (await res.json().catch(() => null)) as { error?: { code?: string } } | null;
          throw new Error(err?.error?.code ?? `http_${res.status}`);
        }
        let done = false;
        for await (const [event, data] of sse(res)) {
          const d = JSON.parse(data);
          if (event === "delta") {
            consumed += d.n;
            opts.onEvent?.({ type: "delta", ...d });
            yield d.text as string;
          } else if (event === "done") {
            state = { fence: d.fence ?? null, mid: !!d.mid };
            done = true;
          }
        }
        if (!done) throw new Error("stream_interrupted");
        if (final && consumed < text.length) {
          // A final request answers everything; if it did not, the reader still gets the end of the message.
          const rest = text.slice(consumed);
          consumed = text.length;
          yield rest;
        }
      } catch (e) {
        if (abort.signal.aborted) return;
        // Fail-open: the rest of this request, then the rest of the stream, passes through untranslated.
        passthrough = true;
        opts.onEvent?.({ type: "fallback", reason: e instanceof Error ? e.message : String(e) });
        const rest = text.slice(consumed);
        consumed = text.length;
        yield rest;
      }
      pending = pending.slice(consumed);
      sent = pending.length;
      wake();
      if (final && pending === "") break;
    }
    if (failure) throw failure;
  } finally {
    abort.abort();
    wake();
    opts.signal?.removeEventListener("abort", onAbort);
    await pump.catch(() => {});
  }
}

/** The translated stream as a `ReadableStream<string>` (pipe it through `TextEncoderStream` for a `Response`). */
export function translateStream(source: Source, opts: LiveOptions): ReadableStream<string> {
  const it = translateIterable(source, opts);
  return new ReadableStream<string>({
    async pull(controller) {
      try {
        const { done, value } = await it.next();
        if (done) controller.close();
        else controller.enqueue(value);
      } catch (e) {
        controller.error(e);
      }
    },
    async cancel() {
      await it.return();
    },
  });
}
