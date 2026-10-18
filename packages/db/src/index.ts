// @glot/db: read translated database content from the shadow table `glot_translations`
// (filled by `glot db sync`). Postgres. No ORM import: Prisma and Drizzle are duck-typed.
import { createHash } from "node:crypto";

/** Translatable columns per table (Prisma: per model), e.g. `{ Product: ["name", "description"] }`. */
export type Localized = Record<string, readonly string[]>;
export type Query = (sql: string, params: unknown[]) => Promise<Record<string, unknown>[]>;
export interface GlotDbOptions {
  localized: Localized;
  /** Primary key column, `id` by default (must match `primary_key` in glot.toml). */
  primaryKey?: string;
}

/** Same hash as `glot_core::hash` and the SQL trigger: first 16 hex chars of sha256. */
export const sourceHash = (text: string): string => createHash("sha256").update(text).digest("hex").slice(0, 16);

const SELECT = `SELECT row_id, "column", value, source_hash FROM glot_translations
  WHERE "table" = $1 AND locale = $2 AND status = 'translated' AND row_id = ANY($3::text[]) AND "column" = ANY($4::text[])`;

/** Core: swap localized columns of `rows` with their `locale` translation, source value as fallback. */
export function createGlotDb(query: Query, opts: GlotDbOptions) {
  const pk = opts.primaryKey ?? "id";
  return {
    async localize<T>(table: string, rows: T[], locale: string): Promise<T[]> {
      const cols = opts.localized[table];
      const ids = [...new Set(rows.map((r) => (r as Record<string, unknown> | null)?.[pk]).filter((v) => v != null).map(String))];
      if (!cols?.length || !ids.length) return rows;
      const found = await query(SELECT, [table, locale, ids, [...cols]]);
      const byCell = new Map(found.map((f) => [`${f.row_id}\0${f.column}`, f]));
      return rows.map((row) => {
        const r = row as Record<string, unknown> | null;
        if (!r || r[pk] == null) return row;
        let out: Record<string, unknown> | null = null;
        for (const col of cols) {
          const src = r[col];
          const t = byCell.get(`${r[pk]}\0${col}`);
          // Hash check: a translation of an older source (not yet marked stale) is never served.
          if (typeof src !== "string" || !t || t.source_hash !== sourceHash(src)) continue;
          (out ??= { ...r })[col] = t.value;
        }
        return (out ?? row) as T;
      });
    },
  };
}

const PRISMA_READS = new Set(["findUnique", "findUniqueOrThrow", "findFirst", "findFirstOrThrow", "findMany"]);

/**
 * Prisma client extension: `const db = prisma.$extends(glotPrisma({ localized: { Product: ["name"] } }))`,
 * then `db.withLocale("fr").product.findMany()`. Keys are model names = table names (no `@@map`).
 * ponytail: top-level rows only; relations loaded with `include` keep their source values.
 */
export function glotPrisma(opts: GlotDbOptions) {
  return {
    name: "glot",
    client: {
      withLocale<C>(this: C, locale: string): C {
        // Untyped on purpose: no @prisma/client import.
        const client = this as any;
        const db = createGlotDb((sql, params) => client.$queryRawUnsafe(sql, ...params), opts);
        return client.$extends({
          query: {
            $allModels: {
              async $allOperations({ model, operation, args, query }: { model: string; operation: string; args: unknown; query: (a: unknown) => Promise<unknown> }) {
                const res = await query(args);
                if (!PRISMA_READS.has(operation) || !opts.localized[model] || res == null) return res;
                if (Array.isArray(res)) return db.localize(model, res, locale);
                return (await db.localize(model, [res], locale))[0];
              },
            },
          },
        });
      },
    },
  };
}

/**
 * Drizzle (node-postgres or postgres.js): `const glot = glotDrizzle(db, { localized: { products: ["name"] } })`,
 * then `await glot.withLocale("fr").localize("products", await db.select().from(products))`.
 */
export function glotDrizzle(db: { $client: unknown }, opts: GlotDbOptions) {
  const c = db.$client as {
    unsafe?: (sql: string, params: unknown[]) => Promise<Record<string, unknown>[]>;
    query?: (sql: string, params: unknown[]) => Promise<{ rows: Record<string, unknown>[] }>;
  };
  const query: Query = c.unsafe ? (sql, p) => c.unsafe!(sql, p) : async (sql, p) => (await c.query!(sql, p)).rows;
  const core = createGlotDb(query, opts);
  return {
    withLocale: (locale: string) => ({
      localize: <T>(table: string, rows: T[]) => core.localize(table, rows, locale),
    }),
  };
}
