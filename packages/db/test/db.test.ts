import { test } from "node:test";
import assert from "node:assert/strict";
import { createGlotDb, glotDrizzle, glotPrisma, sourceHash, type Query } from "../src/index.ts";

const shadow = [
  { row_id: "1", column: "name", value: "Zelt", source_hash: sourceHash("Tent") },
  { row_id: "1", column: "description", value: "Ein leichtes Zelt", source_hash: sourceHash("A light tent") },
  { row_id: "2", column: "name", value: "Kocher", source_hash: sourceHash("Stove") },
];
const calls: unknown[][] = [];
const query: Query = async (_sql, params) => {
  calls.push(params);
  return shadow;
};
const rows = () => [
  { id: 1, name: "Tent", description: "An ultralight tent", price: 10 }, // description edited: old translation not served
  { id: 2, name: "Stove", description: "Boils water" },
  { id: 3, name: "Lamp", description: null },
];

test("sourceHash matches glot_core::hash and the SQL trigger", () => {
  assert.equal(sourceHash("test"), "9f86d081884c7d65");
});

test("localize swaps fresh translations, falls back to source otherwise", async () => {
  calls.length = 0;
  const db = createGlotDb(query, { localized: { Product: ["name", "description"] } });
  const input = rows();
  const out = await db.localize("Product", input, "de");
  assert.deepEqual(out, [
    { id: 1, name: "Zelt", description: "An ultralight tent", price: 10 },
    { id: 2, name: "Kocher", description: "Boils water" },
    { id: 3, name: "Lamp", description: null },
  ]);
  assert.equal(input[0].name, "Tent", "input rows are not mutated");
  assert.deepEqual(calls, [["Product", "de", ["1", "2", "3"], ["name", "description"]]]);
  // Unknown table or no rows: no query.
  assert.equal(await db.localize("Other", input, "de"), input);
  assert.deepEqual(await db.localize("Product", [], "de"), []);
  assert.equal(calls.length, 1);
});

test("glotPrisma localizes reads only, single and list results", async () => {
  let hook: any;
  const base = {
    $queryRawUnsafe: (_sql: string, ...params: unknown[]) => query("", params),
    $extends(ext: any) {
      hook = ext.query.$allModels.$allOperations;
      return "extended";
    },
  };
  const ext = glotPrisma({ localized: { Product: ["name", "description"] } });
  assert.equal(ext.client.withLocale.call(base, "de"), "extended");
  const run = (model: string, operation: string, res: unknown) => hook({ model, operation, args: {}, query: async () => res });
  assert.equal((await run("Product", "findMany", rows()))[1].name, "Kocher");
  assert.equal((await run("Product", "findUnique", rows()[0])).name, "Zelt");
  assert.equal(await run("Product", "findFirst", null), null);
  assert.equal((await run("Product", "update", rows()[0])).name, "Tent");
  assert.equal((await run("User", "findMany", [{ id: 1, name: "Tent" }]))[0].name, "Tent");
});

test("glotDrizzle works with node-postgres and postgres.js clients", async () => {
  const pg = { $client: { query: async (_s: string, p: unknown[]) => ({ rows: await query("", p) }) } };
  const pjs = { $client: { unsafe: (_s: string, p: unknown[]) => query("", p) } };
  for (const db of [pg, pjs]) {
    const out = await glotDrizzle(db, { localized: { products: ["name"] } }).withLocale("de").localize("products", rows());
    assert.equal(out[0].name, "Zelt");
  }
});
