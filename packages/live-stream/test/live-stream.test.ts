import { test } from "node:test";
import assert from "node:assert/strict";
import { translateIterable, translateStream, type LiveEvent } from "../src/index.ts";

type Req = { to: string; text: string; final: boolean; mid: boolean; fence: string | null; conversation?: string; key?: string };

/** The server's contract in miniature: complete sentences come back upper-cased, the rest is left to resend. */
function fakeGlot(opts: { failAt?: number; status?: number; delay?: number } = {}) {
  const requests: Req[] = [];
  const f = async (_url: string | URL | Request, init?: RequestInit) => {
    const b = JSON.parse(String(init!.body)) as Req;
    requests.push(b);
    if (opts.status) return new Response(JSON.stringify({ error: { code: "live_stream_disabled" } }), { status: opts.status });
    if (opts.failAt === requests.length) throw new TypeError("fetch failed");
    if (opts.delay) await new Promise((r) => setTimeout(r, opts.delay));
    if (init?.signal?.aborted) throw new DOMException("aborted", "AbortError");
    const cut = b.final ? b.text.length : Math.max(b.text.lastIndexOf(". ") + 2, b.text.lastIndexOf("\n") + 1, 0);
    const done = b.text.slice(0, cut);
    const events = done.split(/(?<=\. |\n)/).filter(Boolean).map(
      (s) => `event: delta\ndata: ${JSON.stringify({ text: s.toUpperCase(), n: s.length, via: "llm" })}\n\n`,
    );
    events.push(`event: done\ndata: ${JSON.stringify({ consumed: cut, fence: null, mid: cut < b.text.length })}\n\n`);
    // Split the SSE bytes anywhere: the parser must not care.
    const bytes = new TextEncoder().encode(events.join(""));
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        for (let i = 0; i < bytes.length; i += 7) c.enqueue(bytes.slice(i, i + 7));
        c.close();
      },
    });
    return new Response(body, { headers: { "content-type": "text/event-stream" } });
  };
  return { requests, fetch: f as typeof fetch };
}

async function* slowly(parts: string[], ms = 1) {
  for (const p of parts) {
    await new Promise((r) => setTimeout(r, ms));
    yield p;
  }
}

const base = { project: "acme", to: "es", key: "pk_1" };

test("a chunked answer comes out translated, in order, with nothing lost or repeated", async () => {
  const glot = fakeGlot();
  const parts = ["Hel", "lo wor", "ld. How a", "re you? Fine", ". Émoji 👋 ok. By", "e"];
  let out = "";
  for await (const t of translateIterable(slowly(parts), { ...base, fetch: glot.fetch, conversation: "c1" })) out += t;
  assert.equal(out, parts.join("").toUpperCase());
  // Unfinished sentences are sent again with what follows; only the last request is final.
  assert.ok(glot.requests.length >= 2);
  assert.ok(glot.requests.slice(0, -1).every((r) => !r.final));
  assert.equal(glot.requests.at(-1)!.final, true);
  assert.equal(glot.requests.at(-1)!.text.endsWith("Bye"), true);
  assert.ok(glot.requests.every((r) => r.conversation === "c1" && r.key === "pk_1"));
  // No request for text that cannot contain a finished sentence yet.
  assert.notEqual(glot.requests[0].text, "Hel");
});

test("ReadableStream of bytes in, ReadableStream of text out (route handler shape)", async () => {
  const glot = fakeGlot();
  const bytes = new TextEncoder().encode("Première phrase. Deuxième phrase.\n");
  const source = new ReadableStream<Uint8Array>({
    start(c) {
      // é is cut in two: the helper decodes across chunks.
      c.enqueue(bytes.slice(0, 5));
      c.enqueue(bytes.slice(5));
      c.close();
    },
  });
  const res = new Response(translateStream(source, { ...base, fetch: glot.fetch }).pipeThrough(new TextEncoderStream()));
  assert.equal(await res.text(), "PREMIÈRE PHRASE. DEUXIÈME PHRASE.\n");
});

test("fail-open: when glot stops answering the source passes through, complete, and the caller is told", async () => {
  const glot = fakeGlot({ failAt: 2 });
  const events: LiveEvent[] = [];
  let out = "";
  const parts = ["One. ", "Two. ", "Three. ", "Four."];
  for await (const t of translateIterable(slowly(parts, 15), { ...base, fetch: glot.fetch, onEvent: (e) => events.push(e) })) out += t;
  assert.equal(out.toLowerCase(), "one. two. three. four.");
  assert.ok(out.startsWith("ONE. "), "what was translated before the failure stays");
  assert.ok(out.endsWith("Four."), "the rest is the source");
  assert.deepEqual(events.filter((e) => e.type === "fallback"), [{ type: "fallback", reason: "fetch failed" }]);
  assert.equal(glot.requests.length, 2, "glot is not called again for this stream");

  // A refusal (feature off, quota) explains itself the same way.
  const off = fakeGlot({ status: 403 });
  const seen: LiveEvent[] = [];
  let all = "";
  for await (const t of translateIterable(slowly(["Hello. ", "World."]), { ...base, fetch: off.fetch, onEvent: (e) => seen.push(e) })) all += t;
  assert.equal(all, "Hello. World.");
  assert.deepEqual(seen, [{ type: "fallback", reason: "live_stream_disabled" }]);
});

test("cancellation: aborting stops the requests and the reading of the source", async () => {
  const glot = fakeGlot({ delay: 20 });
  const ac = new AbortController();
  let read = 0;
  const source = (async function* () {
    for (let i = 0; i < 200; i++) {
      read++;
      await new Promise((r) => setTimeout(r, 2));
      yield `Sentence ${i}. `;
    }
  })();
  let out = "";
  for await (const t of translateIterable(source, { ...base, fetch: glot.fetch, signal: ac.signal })) {
    out += t;
    ac.abort();
  }
  const requests = glot.requests.length;
  const readAtCancel = read;
  await new Promise((r) => setTimeout(r, 80));
  assert.ok(out.length > 0);
  assert.equal(glot.requests.length, requests);
  assert.ok(read <= readAtCancel + 1 && read < 200, `source still read after cancel (${read})`);
});

test("DOM: text streaming into a container is shown translated, never as source; code is left alone", async () => {
  // The smallest DOM the helper touches.
  type FakeText = { nodeType: 3; data: string; parentElement: { closest: (s: string) => unknown } };
  let notify: (r: unknown[]) => void = () => {};
  const g = globalThis as Record<string, unknown>;
  g.MutationObserver = class {
    constructor(cb: (r: unknown[]) => void) {
      notify = cb;
    }
    observe() {}
    disconnect() {}
  };
  g.NodeFilter = { SHOW_TEXT: 4 };
  g.document = { createTreeWalker: () => ({ nextNode: () => false }) };
  const { observeLive } = await import("../src/dom.ts");
  const glot = fakeGlot();
  const stop = observeLive({} as Element, { ...base, fetch: glot.fetch, settle: 30 });

  const p: FakeText = { nodeType: 3, data: "", parentElement: { closest: () => null } };
  const code: FakeText = { nodeType: 3, data: "", parentElement: { closest: () => ({}) } };
  const shown: string[] = [];
  let full = "";
  for (const part of ["Hello wor", "ld. How are", " you today"]) {
    full += part;
    p.data = full; // the app (React) writes the whole source text
    code.data = "let a = 1. b";
    notify([{ type: "characterData", target: p }, { type: "characterData", target: code }]);
    shown.push(p.data); // what the browser would paint
    await new Promise((r) => setTimeout(r, 10));
  }
  for (const s of shown) assert.ok(!/[a-z]/.test(s), `source text was visible: ${JSON.stringify(s)}`);
  await new Promise((r) => setTimeout(r, 80)); // quiet: the node is complete
  assert.equal(p.data, "HELLO WORLD. HOW ARE YOU TODAY");
  assert.equal(code.data, "let a = 1. b");
  stop();
});
