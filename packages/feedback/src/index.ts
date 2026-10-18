// @glot/feedback — native speakers flag a wrong / awkward / unclear translation in production.
// Select text → "Improve translation", or the runtime badge's "Improve" (`glot:improve` event), or `open()`.
// Sends {locale, text, kind, suggestion, url (no query/fragment), dom (tag path, no text), key?} to
// POST {endpoint}/api/feedback/{key}. No cookie, no storage, no visitor id.
export type Labels = {
  report: string;
  title: string;
  text: string;
  wrong: string;
  awkward: string;
  unclear: string;
  suggestion: string;
  send: string;
  cancel: string;
  thanks: string;
};
export type Options = {
  /** Project public key (dashboard → Feedback, same as Insights). */
  key: string;
  /** glot server origin. */
  endpoint?: string;
  /** Locale the app rendered (default: <html lang>). */
  locale?: string;
  /** Pass them translated in the user's language. */
  labels?: Partial<Labels>;
};

const EN: Labels = {
  report: "Improve translation",
  title: "Is this translation wrong?",
  text: "Text",
  wrong: "Wrong",
  awkward: "Awkward",
  unclear: "Unclear",
  suggestion: "Better wording (optional)",
  send: "Send",
  cancel: "Cancel",
  thanks: "Thanks!",
};

const CSS =
  ":host{all:initial}#p{position:fixed;z-index:2147483646;font:13px system-ui,sans-serif;background:#111;color:#fff;border:0;border-radius:6px;padding:4px 8px;cursor:pointer}" +
  "dialog{font:14px/1.4 system-ui,sans-serif;border:1px solid #ccc;border-radius:10px;padding:16px;width:min(88vw,360px);color:#111;background:#fff}dialog::backdrop{background:#0004}" +
  "fieldset{border:0;padding:0;margin:0 0 8px;display:flex;gap:12px;flex-wrap:wrap}legend{font-weight:600;padding:0;margin-bottom:8px}" +
  "textarea{display:block;width:100%;box-sizing:border-box;font:inherit;margin:4px 0 8px}.r{display:flex;justify-content:flex-end;gap:8px}" +
  "button{font:inherit;padding:6px 12px;border-radius:6px;border:1px solid #888;background:#fff;color:#111;cursor:pointer}#ok{background:#111;color:#fff}" +
  ":focus-visible{outline:2px solid #2563eb;outline-offset:2px}" +
  "@media (prefers-color-scheme:dark){dialog{background:#1c1c1c;color:#eee;border-color:#444}button{background:#2a2a2a;color:#eee}}";

const kinds = ["wrong", "awkward", "unclear"] as const;
const HTML =
  `<style>${CSS}</style><button id="p" type="button" hidden data-l="report"></button>` +
  `<dialog id="d" aria-labelledby="t"><form id="f"><fieldset><legend id="t" data-l="title"></legend>` +
  kinds.map((k, i) => `<label><input type="radio" name="k" value="${k}"${i ? "" : " checked"}> <span data-l="${k}"></span></label>`).join("") +
  `</fieldset><label for="q" data-l="text"></label><textarea id="q" rows="2" maxlength="1000" required></textarea>` +
  `<label for="s" data-l="suggestion"></label><textarea id="s" rows="2" maxlength="1000"></textarea>` +
  `<p id="m" role="status" aria-live="polite"></p><div class="r"><button id="c" type="button" data-l="cancel"></button>` +
  `<button id="ok" data-l="send"></button></div></form></dialog>`;

let o: Options | undefined;
let root: ShadowRoot | undefined;
let cur: { text: string; dom: string; key?: string } = { text: "", dom: "" };

const $ = <T extends HTMLElement>(id: string) => root!.getElementById(id) as T;

/** Tag path of the selected element (no text, no attributes but id): `main>section#pricing>h2`. */
function path(n: Node | null): string {
  const p: string[] = [];
  for (let e = n instanceof Element ? n : n?.parentElement; e && e !== document.body && p.length < 6; e = e.parentElement)
    p.unshift(e.localName + (e.id ? "#" + e.id : ""));
  return p.join(">");
}

function selection(): { text: string; dom: string; rect?: DOMRect } {
  const s = getSelection();
  const text = s?.toString().trim() || "";
  if (!s || !text || text.length > 1000) return { text: "", dom: "" };
  return { text, dom: path(s.anchorNode), rect: s.getRangeAt(0).getBoundingClientRect() };
}

/** Open the form, prefilled with `text` (default: the current selection). */
export function open(text?: string, key?: string): void {
  if (!root) return;
  const sel = selection();
  cur = { text: text ?? (sel.text || cur.text), dom: sel.dom || cur.dom, key };
  $<HTMLTextAreaElement>("q").value = cur.text;
  $<HTMLTextAreaElement>("s").value = "";
  $("m").textContent = "";
  $("p").hidden = true;
  $<HTMLDialogElement>("d").showModal();
}

export function init(opts: Options): void {
  if (root || typeof document === "undefined") return;
  o = opts;
  const host = document.createElement("glot-feedback");
  root = host.attachShadow({ mode: "open" });
  root.innerHTML = HTML;
  const l = { ...EN, ...opts.labels };
  root.querySelectorAll<HTMLElement>("[data-l]").forEach((e) => (e.textContent = l[e.dataset.l as keyof Labels]));
  document.body.append(host);
  const d = $<HTMLDialogElement>("d");
  const pill = $("p");
  pill.onclick = () => open(cur.text);
  $("c").onclick = () => d.close();
  $<HTMLFormElement>("f").onsubmit = (e) => {
    e.preventDefault();
    const text = $<HTMLTextAreaElement>("q").value.trim();
    const body = {
      locale: o!.locale || document.documentElement.lang || "en",
      text,
      kind: (root!.querySelector("input:checked") as HTMLInputElement).value,
      suggestion: $<HTMLTextAreaElement>("s").value.trim() || undefined,
      url: location.origin + location.pathname,
      dom: cur.dom,
      key: text === cur.text ? cur.key : undefined,
    };
    // String body = text/plain: a "simple" CORS request, no preflight.
    fetch(`${o!.endpoint || ""}/api/feedback/${o!.key}`, { method: "POST", body: JSON.stringify(body), keepalive: true }).catch(() => {});
    $("m").textContent = l.thanks;
    setTimeout(() => d.close(), 900);
  };
  const onSelect = (e: Event) => {
    if (e.composedPath().includes(host)) return;
    const s = selection();
    pill.hidden = !s.text;
    if (!s.rect) return;
    cur = { text: s.text, dom: s.dom };
    pill.style.top = Math.min(innerHeight - 32, s.rect.bottom + 6) + "px";
    pill.style.left = Math.max(4, s.rect.left) + "px";
  };
  document.addEventListener("mouseup", onSelect);
  document.addEventListener("keyup", onSelect);
  addEventListener("scroll", () => (pill.hidden = true), { passive: true });
  // "Improve" link of the @glot/runtime badge (detail: {locale, key?, text?}).
  document.addEventListener("glot:improve", (e) => {
    e.preventDefault();
    const x = (e as CustomEvent).detail || {};
    open(x.text, x.key);
  });
}

export const glotFeedback = { init, open };

// <script src="feedback.global.js" data-key="pk_…" data-endpoint="https://glot.example.com"> auto-inits.
const el = typeof document !== "undefined" ? (document.currentScript as HTMLScriptElement | null) : null;
if (el?.dataset.key) {
  (window as unknown as { glotFeedback: typeof glotFeedback }).glotFeedback = glotFeedback;
  const go = () => init({ key: el.dataset.key!, endpoint: el.dataset.endpoint, locale: el.dataset.locale });
  document.body ? go() : addEventListener("DOMContentLoaded", go);
}
