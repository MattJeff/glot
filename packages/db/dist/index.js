import { createHash } from "node:crypto";
const sourceHash = (text) => createHash("sha256").update(text).digest("hex").slice(0, 16);
const SELECT = `SELECT row_id, "column", value, source_hash FROM glot_translations
  WHERE "table" = $1 AND locale = $2 AND status = 'translated' AND row_id = ANY($3::text[]) AND "column" = ANY($4::text[])`;
function createGlotDb(query, opts) {
  const pk = opts.primaryKey ?? "id";
  return {
    async localize(table, rows, locale) {
      const cols = opts.localized[table];
      const ids = [...new Set(rows.map((r) => r?.[pk]).filter((v) => v != null).map(String))];
      if (!cols?.length || !ids.length) return rows;
      const found = await query(SELECT, [table, locale, ids, [...cols]]);
      const byCell = new Map(found.map((f) => [`${f.row_id}\0${f.column}`, f]));
      return rows.map((row) => {
        const r = row;
        if (!r || r[pk] == null) return row;
        let out = null;
        for (const col of cols) {
          const src = r[col];
          const t = byCell.get(`${r[pk]}\0${col}`);
          if (typeof src !== "string" || !t || t.source_hash !== sourceHash(src)) continue;
          (out ??= { ...r })[col] = t.value;
        }
        return out ?? row;
      });
    }
  };
}
const PRISMA_READS = /* @__PURE__ */ new Set(["findUnique", "findUniqueOrThrow", "findFirst", "findFirstOrThrow", "findMany"]);
function glotPrisma(opts) {
  return {
    name: "glot",
    client: {
      withLocale(locale) {
        const client = this;
        const db = createGlotDb((sql, params) => client.$queryRawUnsafe(sql, ...params), opts);
        return client.$extends({
          query: {
            $allModels: {
              async $allOperations({ model, operation, args, query }) {
                const res = await query(args);
                if (!PRISMA_READS.has(operation) || !opts.localized[model] || res == null) return res;
                if (Array.isArray(res)) return db.localize(model, res, locale);
                return (await db.localize(model, [res], locale))[0];
              }
            }
          }
        });
      }
    }
  };
}
function glotDrizzle(db, opts) {
  const c = db.$client;
  const query = c.unsafe ? (sql, p) => c.unsafe(sql, p) : async (sql, p) => (await c.query(sql, p)).rows;
  const core = createGlotDb(query, opts);
  return {
    withLocale: (locale) => ({
      localize: (table, rows) => core.localize(table, rows, locale)
    })
  };
}
export {
  createGlotDb,
  glotDrizzle,
  glotPrisma,
  sourceHash
};
