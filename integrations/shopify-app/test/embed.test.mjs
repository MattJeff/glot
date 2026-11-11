// Renders the app embed with liquidjs (Shopify's `form` tag and `t`/`json` filters stubbed): `pnpm test`.
import { Liquid } from "liquidjs";
import assert from "node:assert/strict";
import fs from "node:fs";

const e = new Liquid();
e.registerFilter("t", (k) => ({ "switcher.label": "Language" })[k]);
e.registerFilter("json", (v) => JSON.stringify(v));
const block = (name, inner) => ({
  parse(_t, rem) {
    this.tpl = [];
    const s = this.liquid.parser.parseStream(rem);
    s.on(`tag:end${name}`, () => s.stop()).on("template", (x) => this.tpl.push(x)).on("end", () => { throw new Error(`no end${name}`); });
    s.start();
  },
  *render(ctx, out) {
    if (!inner) return;
    out.write('<form action="/localization">');
    yield this.liquid.renderer.renderTemplates(this.tpl, ctx, out);
    out.write("</form>");
  },
});
e.registerTag("schema", block("schema", false));
e.registerTag("form", block("form", true));

const src = fs.readFileSync(new URL("../extensions/glot-embed/blocks/glot.liquid", import.meta.url), "utf8");
const langs = [{ iso_code: "en", endonym_name: "english" }, { iso_code: "es", endonym_name: "español" }, { iso_code: "de", endonym_name: "deutsch" }];
const render = (settings) => e.parseAndRender(src, {
  block: { settings: { project: "shop", switcher: "shopify", position: "bottom-left", ...settings } },
  localization: { available_languages: langs, language: langs[1] },
  request: { locale: { iso_code: "es" } },
});

let h = await render({ key: "pk_abcdefgh" });
assert.match(h, /name="language_code"/);
assert.match(h, /<option value="es" selected>Español/);
assert.match(h, /localStorage\.setItem\("glot-locale", "es"\)/); // glot.js follows the store's language
assert.match(h, /data-key="pk_abcdefgh"/);
assert.match(h, /data-switcher="none"/);
assert.match(h, /bottom:16px;left:16px/);

h = await render({ switcher: "glot", key: "glot_secret" });
assert.match(h, /data-position="bottom-left"/);
assert.doesNotMatch(h, /data-key/); // only a pk_ key goes into the page
assert.doesNotMatch(h, /data-switcher=|language_code|localStorage/);

h = await render({ switcher: "none" });
assert.match(h, /data-switcher="none"/);
assert.doesNotMatch(h, /language_code|localStorage/);

h = await render({ project: 'x"><script>alert(1)</script>' });
assert.doesNotMatch(h, /<script>alert/);

assert.equal((await render({ project: "" })).trim(), "");

const schema = JSON.parse(/{% schema %}([\s\S]*){% endschema %}/.exec(src)[1]);
assert.equal(schema.target, "body");
for (const f of ["en.default.schema.json", "fr.schema.json"]) {
  const loc = JSON.parse(fs.readFileSync(new URL(`../extensions/glot-embed/locales/${f}`, import.meta.url), "utf8"));
  for (const [, k] of JSON.stringify(schema).matchAll(/"t:([\w.]+)"/g)) assert.ok(k.split(".").reduce((o, p) => o?.[p], loc), `${f}: ${k}`);
}
console.log("ok");
