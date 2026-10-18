import { test } from "node:test";
import assert from "node:assert/strict";
import { provenanceOf, provenanceLabel, hasMachineTranslations, provenanceBadge } from "../src/provenance.ts";

const cat = {
  "home.title": { origin: "machine", model: "claude-haiku-5-5", translated_at: "2026-09-01T10:00:00Z", reviewed: false },
  "home.cta": { origin: "machine", model: "claude-haiku-5-5", reviewed: true, reviewed_at: "2026-09-03T08:00:00Z" },
  "legal.terms": { origin: "human", reviewed: true, reviewed_at: "2026-09-02T00:00:00Z" },
} as const;

test("labels machine translations only", () => {
  assert.equal(provenanceLabel(provenanceOf(cat, "home.title")), "Translated by AI");
  assert.equal(
    provenanceLabel(provenanceOf(cat, "home.cta"), { locale: "fr", labels: { ai: "Traduit par IA", reviewed: "relu le {date}" } }),
    "Traduit par IA · relu le 3 sept. 2026",
  );
  assert.equal(provenanceLabel(provenanceOf(cat, "legal.terms")), null);
  assert.equal(provenanceLabel(provenanceOf(cat, "nope")), null);
  assert.equal(provenanceOf(cat, "toString"), undefined);
});

test("page-level notice and DOM-less badge", () => {
  assert.equal(hasMachineTranslations(cat), true);
  assert.equal(hasMachineTranslations(cat, ["legal.terms"]), false);
  assert.equal(hasMachineTranslations(undefined), false);
  assert.equal(provenanceBadge(provenanceOf(cat, "home.title")), null, "no document in node");
});
