// Framer plugin (Localization mode): settings saved per project with plugin data, one "Sync" button.
// The glot token is stored in this browser only (localStorage), never in the Framer project.
import "framer-plugin/framer.css";
import { framer } from "framer-plugin";
import { glotClient, sync } from "./sync.ts";

framer.showUI({ position: "top right", width: 300, height: 360 });

const root = document.getElementById("root")!;
root.innerHTML = `
  <form id="f" style="display:flex;flex-direction:column;gap:8px;padding:12px">
    <label>glot server <input name="base" required placeholder="https://app.glotlang.com"></label>
    <label>Project slug <input name="project" required placeholder="my-site"></label>
    <label>API token <input name="token" type="password" required placeholder="glot_…" autocomplete="off"></label>
    <label style="display:flex;gap:6px;align-items:center"><input name="translate" type="checkbox" checked> Translate missing strings</label>
    <label style="display:flex;gap:6px;align-items:center"><input name="force" type="checkbox"> Overwrite values marked done</label>
    <button class="framer-button-primary" type="submit">Sync with glot</button>
    <p id="out" role="status" style="margin:0;white-space:pre-wrap"></p>
  </form>`;
const form = root.querySelector<HTMLFormElement>("#f")!;
const out = root.querySelector<HTMLParagraphElement>("#out")!;
const field = (n: string) => form.elements.namedItem(n) as HTMLInputElement;

const saved = JSON.parse((await framer.getPluginData("glot")) ?? "{}");
field("base").value = saved.base ?? "https://app.glotlang.com";
field("project").value = saved.project ?? "";
try {
  field("token").value = localStorage.getItem("glot.token") ?? "";
} catch {}

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  if (!framer.isAllowedTo("setLocalizationData")) {
    out.textContent = "You need edit permission on this project's localization.";
    return;
  }
  const base = field("base").value.trim(),
    project = field("project").value.trim(),
    token = field("token").value.trim();
  await framer.setPluginData("glot", JSON.stringify({ base, project }));
  try {
    localStorage.setItem("glot.token", token);
  } catch {}
  const button = form.querySelector("button")!;
  button.disabled = true;
  try {
    // `/api/plugin`: the token-only, CORS-enabled glot routes a browser plugin can call.
    const glot = glotClient({ base, token, project, prefix: "/api/plugin" });
    const r = await sync(framer, glot, {
      translate: field("translate").checked,
      force: field("force").checked,
      onProgress: (m) => (out.textContent = m),
    });
    out.textContent =
      `${r.sources} strings, ${r.written} translations written, ${r.skipped} skipped.` +
      (r.unmatchedLocales.length ? `\nNo glot locale for: ${r.unmatchedLocales.join(", ")}.` : "") +
      (r.errors.length ? `\n${r.errors.join("\n")}` : "");
    framer.notify(`glot: ${r.written} translations written`, { variant: r.errors.length ? "warning" : "success" });
  } catch (err) {
    out.textContent = err instanceof Error ? err.message : String(err);
  } finally {
    button.disabled = false;
  }
});
