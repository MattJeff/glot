// Small "translated on your device" badge. Plain DOM, no framework.
export interface BadgeOptions {
  /** Badge text (pass it translated — e.g. from your fallback messages). */
  label?: string;
  /** "Improve" link text. */
  improveLabel?: string;
  /** Where "Improve" goes (glot overlay / feedback form). Without it, the link only dispatches `glot:improve`. */
  improveUrl?: string;
  locale?: string;
  /** Model download progress 0..1 while preparing; the badge shows a percentage. */
  progress?: number;
  /** Shown when the model must be downloaded: clicking calls `onActivate` (inside the user gesture). */
  activateLabel?: string;
  onActivate?: () => void;
}

const ID = "glot-runtime-badge";

/** Show/update the badge. Returns a remover. No-op without a DOM. */
export function showBadge(o: BadgeOptions = {}): () => void {
  if (typeof document === "undefined") return () => {};
  let el = document.getElementById(ID);
  if (!el) {
    el = document.createElement("div");
    el.id = ID;
    el.setAttribute("role", "status");
    el.style.cssText =
      "position:fixed;bottom:12px;inset-inline-end:12px;z-index:2147483646;font:12px/1.4 system-ui,sans-serif;" +
      "background:#111c;color:#fff;padding:6px 10px;border-radius:8px;display:flex;gap:8px;align-items:center;backdrop-filter:blur(4px)";
    document.body.append(el);
  }
  el.replaceChildren();
  if (o.locale) el.lang = o.locale;
  const span = document.createElement("span");
  span.textContent = o.onActivate
    ? (o.activateLabel ?? "Translate this page on your device")
    : (o.label ?? "Automatically translated on your device") + (o.progress !== undefined && o.progress < 1 ? ` · ${Math.round(o.progress * 100)}%` : "");
  el.append(span);
  const link = (text: string, onClick: (e: MouseEvent) => void, href = "#") => {
    const a = document.createElement("a");
    a.textContent = text;
    a.href = href;
    a.style.cssText = "color:inherit;text-decoration:underline;cursor:pointer";
    a.addEventListener("click", onClick);
    el!.append(a);
  };
  if (o.onActivate) {
    span.style.cursor = "pointer";
    span.addEventListener("click", o.onActivate);
    span.setAttribute("role", "button");
    span.tabIndex = 0;
  } else {
    link(o.improveLabel ?? "Improve", (e) => {
      const ok = el!.dispatchEvent(new CustomEvent("glot:improve", { bubbles: true, cancelable: true, detail: { locale: o.locale } }));
      if (!ok || !o.improveUrl) e.preventDefault();
    }, o.improveUrl);
  }
  return () => el?.remove();
}
