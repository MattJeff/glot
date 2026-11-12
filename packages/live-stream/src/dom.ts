// DOM helper: translate the text that streams into a container (a chat answer being rendered), text node by
// text node. The app keeps writing its own text; the reader only ever sees translated, final text.
//
//   const stop = observeLive(document.querySelector("#answer")!, { project: "acme", key: "pk_…", to: "es" });
//
// How: a MutationObserver sees the app's writes before the browser paints. Each text node is a small stream:
// when its text grows, the new part goes to `translateIterable`; the node shows what is translated so far.
// Text inside <pre>, <code>, <script>, <style>, [translate="no"] and .notranslate is left alone.
// A node whose text stops growing for `settle` ms is finished (its unfinished sentence is translated).
// ponytail: text nodes only, one stream per node; a sentence split across inline elements (<b>, <a>) is translated
// in parts. Translate at the source with `translateStream` when you control the stream.
import { translateIterable, type LiveOptions } from "./index.ts";

export type ObserveOptions = Omit<LiveOptions, "signal"> & {
  /** Quiet time after which a node's text is considered complete. Default 700 ms. */
  settle?: number;
};

type Feed = { push: (s: string) => void; end: () => void; timer?: ReturnType<typeof setTimeout> };
type Track = { source: string; shown: string; feed?: Feed };

const SKIP = "pre,code,script,style,textarea,[translate=no],.notranslate,[contenteditable]";

export function observeLive(root: Element, opts: ObserveOptions): () => void {
  const tracks = new WeakMap<Text, Track>();
  const abort = new AbortController();
  const settle = opts.settle ?? 700;

  const open = (node: Text, t: Track): Feed => {
    const queue: string[] = [];
    let ended = false;
    let wake = () => {};
    const source = (async function* () {
      for (;;) {
        while (queue.length) yield queue.shift()!;
        if (ended) return;
        await new Promise<void>((r) => (wake = r));
      }
    })();
    void (async () => {
      for await (const out of translateIterable(source, { ...opts, signal: abort.signal })) {
        t.shown += out;
        if (node.data !== t.shown) node.data = t.shown;
      }
    })().catch(() => {});
    return {
      push: (s) => (queue.push(s), wake()),
      end: () => ((ended = true), wake()),
    };
  };

  const see = (node: Text) => {
    if (!node.parentElement || node.parentElement.closest(SKIP)) return;
    const now = node.data;
    let t = tracks.get(node);
    if (t && now === t.shown) return; // our own write
    if (!t || !now.startsWith(t.source)) {
      // New node, or the app rewrote it: start over.
      t?.feed?.end();
      t = { source: "", shown: "" };
      tracks.set(node, t);
    }
    const added = now.slice(t.source.length);
    t.source = now;
    node.data = t.shown; // before paint: the source is never visible
    if (!added) return;
    const feed = (t.feed ??= open(node, t));
    feed.push(added);
    clearTimeout(feed.timer);
    const track = t;
    feed.timer = setTimeout(() => {
      feed.end();
      track.feed = undefined;
    }, settle);
  };

  const walk = (n: Node) => {
    if (n.nodeType === 3) return see(n as Text);
    const w = document.createTreeWalker(n, NodeFilter.SHOW_TEXT);
    while (w.nextNode()) see(w.currentNode as Text);
  };

  const mo = new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === "characterData") see(r.target as Text);
      else r.addedNodes.forEach(walk);
    }
  });
  mo.observe(root, { subtree: true, childList: true, characterData: true });
  walk(root);
  return () => {
    mo.disconnect();
    abort.abort();
  };
}
