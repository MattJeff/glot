import { test } from "node:test";
import assert from "node:assert/strict";
import { translatePayloads } from "../src/hydration.ts";

const ES: Record<string, string> = { "Simple pricing for teams": "Precios simples para equipos", "Add to cart": "Añadir al carrito", Pro: "Profesional", "Long story": "Historia <larga>" };
const es = (s: string) => ES[s];

test("__NEXT_DATA__: values translated, identifier-like keys kept", () => {
  const h = `<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"title":"Simple pricing for teams","plan":"Pro","list":["Add to cart"]}},"page":"/p"}</script>`;
  const { html, replaced } = translatePayloads(h, es);
  assert.equal(replaced, 2);
  assert.match(html, /"title":"Precios simples para equipos"/);
  assert.match(html, /"plan":"Pro"/);
});

test("__NUXT_DATA__ devalue array", () => {
  const h = `<script type="application/json" id="__NUXT_DATA__" data-ssr="true">[["ShallowReactive",1],{"data":2,"state":4},{"title":3,"variant":5},"Simple pricing for teams",{},"Pro"]</script>`;
  const { html, replaced } = translatePayloads(h, es);
  assert.equal(replaced, 1);
  assert.match(html, /"Precios simples para equipos"/);
  assert.match(html, /,"Pro"\]/);
});

test("RSC flight rows: JSON rows translated and escaped, T rows and hrefs untouched", () => {
  const chunk = '1:"$Sreact.fragment"\n7:["$","h1",null,{"children":"Simple pricing for teams"}]\n8:T6,Add to9:["$","$L9",null,{"label":"Long story","href":"Add to cart"}]\n';
  const h = `<script>self.__next_f.push([0])</script><script>self.__next_f.push(${JSON.stringify([1, chunk])})</script>`;
  const { html, replaced } = translatePayloads(h, es);
  assert.equal(replaced, 2);
  assert.ok(!html.includes("<larga>"), "escaped inside <script>");
  const lit = /push\((\[1,[\s\S]*?\])\)<\/script>/.exec(html)![1];
  const s = JSON.parse(lit)[1] as string;
  assert.match(s, /\{"children":"Precios simples para equipos"\}/);
  assert.match(s, /8:T6,Add to9:/);
  assert.match(s, /"label":"Historia <larga>","href":"Add to cart"/);
});

test("tmInline: small TM inlined (script-safe), larger than TM_INLINE_MAX bytes → nothing", async () => {
  const { tmInline, TM_INLINE_MAX } = await import("../src/hydration.ts");
  assert.equal(tmInline(`{"v":1,"t":{"a":"</script>"}}`), `<script type="application/json" id="glot-tm">{"v":1,"t":{"a":"\\u003c/script\\u003e"}}</script>`);
  assert.equal(tmInline(`"${"é".repeat(TM_INLINE_MAX / 2)}"`), ""); // bytes, not characters
});
