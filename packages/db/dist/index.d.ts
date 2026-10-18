/** Translatable columns per table (Prisma: per model), e.g. `{ Product: ["name", "description"] }`. */
export type Localized = Record<string, readonly string[]>;
export type Query = (sql: string, params: unknown[]) => Promise<Record<string, unknown>[]>;
export interface GlotDbOptions {
    localized: Localized;
    /** Primary key column, `id` by default (must match `primary_key` in glot.toml). */
    primaryKey?: string;
}
/** Same hash as `glot_core::hash` and the SQL trigger: first 16 hex chars of sha256. */
export declare const sourceHash: (text: string) => string;
/** Core: swap localized columns of `rows` with their `locale` translation, source value as fallback. */
export declare function createGlotDb(query: Query, opts: GlotDbOptions): {
    localize<T>(table: string, rows: T[], locale: string): Promise<T[]>;
};
/**
 * Prisma client extension: `const db = prisma.$extends(glotPrisma({ localized: { Product: ["name"] } }))`,
 * then `db.withLocale("fr").product.findMany()`. Keys are model names = table names (no `@@map`).
 * ponytail: top-level rows only; relations loaded with `include` keep their source values.
 */
export declare function glotPrisma(opts: GlotDbOptions): {
    name: string;
    client: {
        withLocale<C>(this: C, locale: string): C;
    };
};
/**
 * Drizzle (node-postgres or postgres.js): `const glot = glotDrizzle(db, { localized: { products: ["name"] } })`,
 * then `await glot.withLocale("fr").localize("products", await db.select().from(products))`.
 */
export declare function glotDrizzle(db: {
    $client: unknown;
}, opts: GlotDbOptions): {
    withLocale: (locale: string) => {
        localize: <T>(table: string, rows: T[]) => Promise<T[]>;
    };
};
