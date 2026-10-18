import { IntlMessageFormat } from "intl-messageformat";
import { fallbackWithBadge, isSupported } from "@glot/runtime";
import en from "./messages/en.json";
import fr from "./messages/fr.json";
import glossary from "./glossary.json";

const shipped = { en, fr };
const params = new URLSearchParams(location.search);
if (params.has("fake")) await import("./fake-translator.js");

const sel = document.querySelector("#locale");
for (const l of ["en", "fr", "de", "es", "pl", "ja", "it", "pt"]) sel.add(new Option(shipped[l] ? `${l} (shipped)` : `${l} (on device)`, l));
sel.value = params.get("locale") ?? navigator.language.split("-")[0];
sel.onchange = () => { params.set("locale", sel.value); location.search = params; };

const status = (t) => (document.querySelector("#status").textContent = t);

function render(messages, locale) {
  // DOM nodes, never innerHTML: translated text is untrusted.
  const t = (key, values) => {
    const msg = key.split(".").reduce((o, k) => o?.[k], messages);
    return [].concat(new IntlMessageFormat(msg, locale).format(values));
  };
  const el = (tag, parts) => { const e = document.createElement(tag); e.append(...parts); return e; };
  const b = (chunks) => el("b", chunks);
  document.querySelector("#app").replaceChildren(
    el("h1", t("home.title", { name: "Ana" })),
    el("p", t("home.files", { n: 3, folder: "Design" })),
    el("p", t("home.files", { n: 1, folder: "Design" })),
    el("p", t("home.invite", { b })),
    el("button", t("home.cta")),
  );
  document.documentElement.lang = locale;
}

const locale = sel.value;
render(shipped[locale] ?? en, shipped[locale] ? locale : "en");
if (!shipped[locale]) {
  if (!isSupported()) status("Translator API not available here: staying in English.");
  else {
    status("Translating on your device…");
    const r = await fallbackWithBadge({
      source: en, glossary, locale,
      badge: { improveUrl: "https://github.com/glot-i18n/glot/issues/new" },
      onMessages: (m, res) => {
        render(m, locale);
        status(`${res.translated} strings translated (${res.cached} from cache), ${res.fellBack.length} kept in English: ${res.fellBack.join(", ") || "none"}`);
      },
    });
    if (r.status !== "translated") status(`Status: ${r.status}`);
  }
}
