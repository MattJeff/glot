# @glot-i18n/db

Localized database content for Postgres with Prisma or Drizzle: reads the `glot_translations` shadow table
(filled by `glot db sync`) and falls back to the source value when a translation is missing or stale.

```sh
npm i @glot-i18n/db
psql "$DATABASE_URL" -f node_modules/@glot-i18n/db/migrations/0001_glot_translations.sql
```

```ts
// Prisma
import { glotPrisma } from "@glot-i18n/db";
const db = prisma.$extends(glotPrisma({ localized: { Product: ["name", "description"] } }));
await db.withLocale("fr").product.findMany();

// Drizzle (node-postgres or postgres.js)
import { glotDrizzle } from "@glot-i18n/db";
const glot = glotDrizzle(db, { localized: { products: ["name"] } });
await glot.withLocale("fr").localize("products", await db.select().from(products));
```

MIT — part of [glot](https://github.com/MattJeff/glot).
