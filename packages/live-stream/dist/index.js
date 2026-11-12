// src/index.ts
async function* chunks(source) {
  const dec = new TextDecoder();
  const text = (c) => typeof c === "string" ? c : dec.decode(c, { stream: true });
  if (Symbol.asyncIterator in source) {
    for await (const c of source) yield text(c);
    return;
  }
  const reader = source.getReader();
  try {
    for (; ; ) {
      const { done, value } = await reader.read();
      if (done) return;
      yield text(value);
    }
  } finally {
    void reader.cancel().catch(() => {
    });
  }
}
async function* sse(res) {
  const reader = res.body.getReader();
  const dec = new TextDecoder();
  let buf = "";
  for (; ; ) {
    const { done, value } = await reader.read();
    if (done) return;
    buf += dec.decode(value, { stream: true });
    for (let end; (end = buf.indexOf("\n\n")) >= 0; buf = buf.slice(end + 2)) {
      let event = "message";
      const data = [];
      for (const line of buf.slice(0, end).split("\n")) {
        if (line.startsWith("event:")) event = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
      }
      if (data.length) yield [event, data.join("\n")];
    }
  }
}
var HIGH_WATER = 16e3;
async function* translateIterable(source, opts) {
  const doFetch = opts.fetch ?? fetch;
  const url = opts.url ?? `${opts.endpoint ?? "https://api.glotlang.com"}${opts.token ? `/api/projects/${opts.project}/live-stream/translate` : `/live/v1/${opts.project}/translate`}`;
  const abort = new AbortController();
  const onAbort = () => abort.abort();
  opts.signal?.addEventListener("abort", onAbort);
  let pending = "";
  let ended = false;
  let failure;
  let wake = () => {
  };
  const sleep = () => new Promise((r) => wake = r);
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
  let state = { fence: null, mid: false };
  let passthrough = false;
  let sent = 0;
  try {
    for (; ; ) {
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
      let consumed = 0;
      try {
        const res = await doFetch(url, {
          method: "POST",
          // text/plain from browsers: no CORS preflight (the body is JSON either way).
          headers: opts.token ? { "content-type": "application/json", authorization: `Bearer ${opts.token}` } : { "content-type": opts.url ? "application/json" : "text/plain" },
          body: JSON.stringify({
            to: opts.to,
            from: opts.from,
            text,
            final,
            conversation: opts.conversation,
            fence: state.fence,
            mid: state.mid,
            key: opts.key
          }),
          signal: abort.signal
        });
        if (!res.ok || !res.body) {
          const err = await res.json().catch(() => null);
          throw new Error(err?.error?.code ?? `http_${res.status}`);
        }
        let done = false;
        for await (const [event, data] of sse(res)) {
          const d = JSON.parse(data);
          if (event === "delta") {
            consumed += d.n;
            opts.onEvent?.({ type: "delta", ...d });
            yield d.text;
          } else if (event === "done") {
            state = { fence: d.fence ?? null, mid: !!d.mid };
            done = true;
          }
        }
        if (!done) throw new Error("stream_interrupted");
        if (final && consumed < text.length) {
          const rest = text.slice(consumed);
          consumed = text.length;
          yield rest;
        }
      } catch (e) {
        if (abort.signal.aborted) return;
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
    await pump.catch(() => {
    });
  }
}
function translateStream(source, opts) {
  const it = translateIterable(source, opts);
  return new ReadableStream({
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
    }
  });
}
export {
  translateIterable,
  translateStream
};
