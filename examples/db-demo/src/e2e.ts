// End to end, offline (mock LLM): seed 30 products → glot db sync → withLocale("de") → edit → stale → resync.
// ADMIN_URL=postgres://glot:glot@localhost:5433/postgres GLOT_BIN=../../target/integ/debug/glot pnpm e2e
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { PrismaClient } from "@prisma/client";
import { glotPrisma } from "@glot/db";
import { seed } from "./seed.ts";

const admin = process.env.ADMIN_URL ?? "postgres://glot:glot@localhost:5433/postgres";
const glot = process.env.GLOT_BIN ?? "glot";
const name = `glot_dbl10n_demo_${process.pid}`;
const url = `${admin.slice(0, admin.lastIndexOf("/"))}/${name}`;
const env = { ...process.env, DATABASE_URL: url };
const sh = (cmd: string, args: string[]) => execFileSync(cmd, args, { env, encoding: "utf8" });

const adminDb = new PrismaClient({ datasourceUrl: admin });
await adminDb.$executeRawUnsafe(`CREATE DATABASE ${name}`);
const prisma = new PrismaClient({ datasourceUrl: url });
try {
  sh("npx", ["prisma", "db", "push", "--skip-generate"]);
  console.log("seeded", await seed(prisma), "products");

  sh(glot, ["db", "install"]); // glot_translations + trigger (marks stale, NOTIFY)
  console.log(sh(glot, ["db", "sync", "--database-url", url]));

  const db = prisma.$extends(glotPrisma({ localized: { Product: ["name", "description"] } }));
  const de = db.withLocale("de");
  const tent = await de.product.findFirstOrThrow({ where: { name: "Ultralight Tent" } });
  assert.equal(tent.name, "⟦de⟧ Ultraleicht Tent"); // glossary term applied
  assert.equal(tent.description, "⟦de⟧ A two-person tent that weighs under one kilo.");
  const all = await de.product.findMany();
  assert.equal(all.length, 30);
  assert.ok(all.every((p) => p.name.startsWith("⟦de⟧")));
  assert.equal((await db.product.findFirstOrThrow({ where: { id: tent.id } })).name, "Ultralight Tent"); // plain client untouched

  // Idempotent: a second sync translates nothing.
  assert.match(sh(glot, ["db", "sync"]), /^cost: \$0\.0000\n$/);

  // Edit a description: the trigger marks it stale and readers fall back to the new source at once.
  await prisma.product.update({ where: { id: tent.id }, data: { description: "Now with a bigger porch." } });
  const stale = await prisma.$queryRaw<{ n: bigint }[]>`SELECT count(*) AS n FROM glot_translations WHERE status = 'stale'`;
  assert.equal(Number(stale[0].n), 2); // de + fr
  assert.equal((await de.product.findUniqueOrThrow({ where: { id: tent.id } })).description, "Now with a bigger porch.");

  const out = sh(glot, ["db", "sync", "--locales", "de"]);
  assert.match(out, /Product → de: 1 cell\(s\) to translate/);
  assert.equal((await de.product.findUniqueOrThrow({ where: { id: tent.id } })).description, "⟦de⟧ Now with a bigger porch.");
  assert.equal((await db.withLocale("fr").product.findUniqueOrThrow({ where: { id: tent.id } })).description, "Now with a bigger porch.");
  console.log("e2e ok");
} finally {
  await prisma.$disconnect();
  await adminDb.$executeRawUnsafe(`DROP DATABASE IF EXISTS ${name} WITH (FORCE)`);
  await adminDb.$disconnect();
}
