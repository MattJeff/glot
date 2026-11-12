// <script src="https://glotlang.com/widget/glot-live.js" data-project="acme" data-key="pk_…" defer></script>
// then mark what streams: <div data-glot-live="es">…</div> (or call GlotLive.observe(el, { to: "es" })).
// `data-glot-live=""` uses the page language of the visitor (navigator.language).
import { observeLive, type ObserveOptions } from "./dom.ts";

const script = document.currentScript as HTMLScriptElement | null;
const base = {
  project: script?.dataset.project ?? "",
  key: script?.dataset.key,
  endpoint: script?.dataset.endpoint,
};

const observe = (el: Element, o: Partial<ObserveOptions> & { to: string }) => observeLive(el, { ...base, ...o });

const start = () =>
  document.querySelectorAll<HTMLElement>("[data-glot-live]").forEach((el) => {
    observe(el, {
      to: el.dataset.glotLive || navigator.language,
      from: el.dataset.glotFrom,
      conversation: el.dataset.glotConversation,
    });
  });

(globalThis as unknown as { GlotLive: unknown }).GlotLive = { observe };
if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
else start();
