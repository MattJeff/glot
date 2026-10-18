import { test } from "node:test";
import assert from "node:assert/strict";
import { withVariants } from "../src/variants.ts";

test("withVariants overrides existing string keys only, without mutating", () => {
  const m = { pricing: { cta: "Commencer", title: "Tarifs" }, other: "x" };
  const out = withVariants(m, { "pricing.cta": "Essayez gratuitement", "pricing.nope": "y", "other.deep": "z" });
  assert.deepEqual(out, { pricing: { cta: "Essayez gratuitement", title: "Tarifs" }, other: "x" });
  assert.equal(m.pricing.cta, "Commencer");
  assert.equal(withVariants(m, {}), m, "same object when nothing applies: no re-render");
  assert.equal(withVariants(m, { "pricing.nope": "y" }), m);
});
