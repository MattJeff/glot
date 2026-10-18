import { test } from "node:test";
import assert from "node:assert/strict";
import { createGlotDevTranslator, encodeKey, getGlotMessages } from "../src/dev.ts";

test("getGlotMessages: off by default, marks every string with its dotted key when on", () => {
  const msgs = { nav: { save: "Save", list: ["a"] }, n: 1 };
  assert.equal(getGlotMessages(msgs), msgs);
  const on = getGlotMessages(msgs, true);
  assert.equal(on.nav.save, encodeKey("nav.save") + "Save");
  assert.equal(on.nav.list[0], encodeKey("nav.list.0") + "a");
  assert.equal(on.n, 1);
  // Zero-width: only the marker's own code points, no visible characters.
  assert.match(encodeKey("x.y"), /^⁣[\u{FE00}-\u{FE0F}\u{E0100}-\u{E01EF}]+⁣$/u);
  const t = createGlotDevTranslator((k: string) => k.toUpperCase(), "ns", true);
  assert.equal(t("save"), encodeKey("ns.save") + "SAVE");
});
