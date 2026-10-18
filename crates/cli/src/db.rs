//! `glot db`: translate database content (product catalog, help articles, plans, notifications) into the
//! shadow table `glot_translations`, read back by `@glot/db` (`withLocale`). Postgres only.
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::Deserialize;
use sqlx::{PgPool, Row as _};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::ExitCode;

use glot_core::{
    config::{CONFIG_FILE, Config},
    context::KeyContext,
    glossary::{GLOSSARY_FILE, Glossary},
    hash::hash,
    validate,
};
use glot_llm::{
    provider,
    translate::{self, Item, Request, Translator, Usage},
};

/// Shadow table + change trigger. Same text as `packages/db/migrations/0001_glot_translations.sql` (tested).
/// The trigger marks translations `stale` when a localized column's hash changes (same 16-hex sha256 prefix
/// as `glot_core::hash`), drops them when the row is deleted, and wakes `glot db sync --watch` (NOTIFY glot_db).
pub const SCHEMA: &str = r#"CREATE TABLE IF NOT EXISTS glot_translations (
  "table"     text NOT NULL,
  row_id      text NOT NULL,
  "column"    text NOT NULL,
  locale      text NOT NULL,
  value       text NOT NULL,
  source_hash text NOT NULL,
  status      text NOT NULL DEFAULT 'translated', -- translated | stale | failed
  model       text,
  updated_at  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("table", row_id, "column", locale)
);

-- Trigger arguments: primary key column, then the localized columns.
CREATE OR REPLACE FUNCTION glot_db_changed() RETURNS trigger AS $$
DECLARE
  pk text := TG_ARGV[0];
  col text;
  changed boolean := TG_OP = 'INSERT';
BEGIN
  IF TG_OP = 'DELETE' THEN
    DELETE FROM glot_translations WHERE "table" = TG_TABLE_NAME AND row_id = to_jsonb(OLD) ->> pk;
    RETURN OLD;
  END IF;
  FOR i IN 1 .. TG_NARGS - 1 LOOP
    col := TG_ARGV[i];
    IF TG_OP = 'UPDATE' AND (to_jsonb(NEW) ->> col) IS DISTINCT FROM (to_jsonb(OLD) ->> col) THEN
      changed := true;
      UPDATE glot_translations SET status = 'stale', updated_at = now()
       WHERE "table" = TG_TABLE_NAME AND row_id = to_jsonb(NEW) ->> pk AND "column" = col AND status <> 'stale'
         AND source_hash <> left(encode(sha256(convert_to(coalesce(to_jsonb(NEW) ->> col, ''), 'UTF8')), 'hex'), 16);
    END IF;
  END LOOP;
  IF changed THEN
    PERFORM pg_notify('glot_db', TG_TABLE_NAME);
  END IF;
  RETURN NEW;
END $$ LANGUAGE plpgsql;
"#;

#[derive(Args)]
pub struct DbArgs {
    #[command(subcommand)]
    cmd: DbCmd,
}

#[derive(Subcommand)]
enum DbCmd {
    /// Create glot_translations and the change triggers on every [db] table (idempotent).
    Install {
        /// Postgres URL (default: $DATABASE_URL).
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Translate missing and stale database content into glot_translations.
    Sync {
        /// Postgres URL (default: $DATABASE_URL).
        #[arg(long)]
        database_url: Option<String>,
        /// Locales to translate (comma separated). Default: all targets.
        #[arg(long, value_delimiter = ',')]
        locales: Vec<String>,
        /// Show what would be translated and the estimated cost only.
        #[arg(long)]
        dry_run: bool,
        /// Keep running: resync on each change notified by the trigger (`glot db install`).
        #[arg(long)]
        watch: bool,
    },
}

/// `[db]` section of glot.toml.
#[derive(Debug, Default, Deserialize)]
pub struct DbConfig {
    #[serde(default)]
    pub tables: Vec<TableCfg>,
}

#[derive(Debug, Deserialize)]
pub struct TableCfg {
    /// Table name as Postgres sees it (Prisma: the model name unless `@@map`).
    pub table: String,
    #[serde(default = "default_pk")]
    pub primary_key: String,
    /// What the rows are, e.g. "Products of an outdoor gear shop".
    pub context: Option<String>,
    /// Localized columns: `name = "Product name"` or `name = { context = "…", max_length = 60 }`.
    pub columns: BTreeMap<String, ColumnCfg>,
    /// Non-translated columns shown to the LLM as context (e.g. `category`).
    #[serde(default)]
    pub context_columns: Vec<String>,
}

fn default_pk() -> String {
    "id".into()
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ColumnCfg {
    Note(String),
    Full {
        context: Option<String>,
        max_length: Option<usize>,
    },
}

/// One source row: primary key + every selected column (localized and context).
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub values: BTreeMap<String, String>,
}

/// Shadow state of one (row_id, column, locale).
#[derive(Debug, Clone)]
pub struct Existing {
    pub source_hash: String,
    pub status: String,
    pub value: String,
}

pub type Shadow = HashMap<(String, String, String), Existing>;

/// A cell to translate.
#[derive(Debug, Clone)]
pub struct Todo {
    pub item: Item,
    pub row_id: String,
    pub column: String,
    pub source_hash: String,
}

/// Cells of `t` that need a `locale` translation: missing, stale (hash changed), or stale-marked.
/// Up-to-date and failed-with-same-source cells are skipped (idempotent, no money burnt in `--watch`).
pub fn select(t: &TableCfg, rows: &[Row], locale: &str, shadow: &Shadow) -> Vec<Todo> {
    let mut out = vec![];
    for row in rows {
        for (col, cfg) in &t.columns {
            let Some(source) = row.values.get(col).filter(|v| !v.trim().is_empty()) else {
                continue;
            };
            let h = hash(source);
            let existing = shadow.get(&(row.id.clone(), col.clone(), locale.to_string()));
            if existing.is_some_and(|e| e.source_hash == h && e.status != "stale") {
                continue;
            }
            let (note, max_length) = match cfg {
                ColumnCfg::Note(n) => (Some(n.as_str()), None),
                ColumnCfg::Full {
                    context,
                    max_length,
                } => (context.as_deref(), *max_length),
            };
            let mut description = format!(
                "Database content: column \"{col}\" of table \"{}\"",
                t.table
            );
            if let Some(c) = &t.context {
                description += &format!(" ({c})");
            }
            if let Some(n) = note {
                description += &format!(". {n}");
            }
            // Neighbors = the rest of the row (category, title next to a description…), capped.
            let neighbors = t
                .context_columns
                .iter()
                .chain(t.columns.keys())
                .filter(|c| *c != col)
                .filter_map(|c| {
                    let v = row.values.get(c).filter(|v| !v.trim().is_empty())?;
                    Some((c.clone(), v.chars().take(200).collect()))
                })
                .collect();
            let key = format!("{}/{}/{col}", t.table, row.id);
            out.push(Todo {
                item: Item {
                    source: source.clone(),
                    context: KeyContext {
                        key: key.clone(),
                        description: Some(description),
                        max_length,
                        neighbors,
                        ..Default::default()
                    },
                    previous: existing.map(|e| e.value.clone()).filter(|v| !v.is_empty()),
                    key,
                },
                row_id: row.id.clone(),
                column: col.clone(),
                source_hash: h,
            });
        }
    }
    out
}

/// `"name"` with embedded quotes doubled.
fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn trigger_sql(t: &TableCfg) -> String {
    let args: Vec<String> = std::iter::once(&t.primary_key)
        .chain(t.columns.keys())
        .map(|a| literal(a))
        .collect();
    format!(
        "DROP TRIGGER IF EXISTS glot_db ON {tbl};\n\
         CREATE TRIGGER glot_db AFTER INSERT OR UPDATE OR DELETE ON {tbl} \
         FOR EACH ROW EXECUTE FUNCTION glot_db_changed({});",
        args.join(", "),
        tbl = ident(&t.table)
    )
}

pub fn load(root: &Path) -> Result<(Config, DbConfig)> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        db: DbConfig,
    }
    let cfg = Config::load(root)?;
    let text = std::fs::read_to_string(root.join(CONFIG_FILE))?;
    let db = toml::from_str::<File>(&text)
        .with_context(|| format!("[db] section of {CONFIG_FILE}"))?
        .db;
    if db.tables.is_empty() {
        bail!("no [[db.tables]] in {CONFIG_FILE}; see README \"Database content\"");
    }
    Ok((cfg, db))
}

async fn connect(url: Option<String>) -> Result<PgPool> {
    let url = url
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .context("pass --database-url or set DATABASE_URL")?;
    PgPool::connect(&url)
        .await
        .context("connecting to Postgres")
}

pub async fn install(pool: &PgPool, db: &DbConfig) -> Result<()> {
    sqlx::raw_sql(SCHEMA).execute(pool).await?;
    for t in &db.tables {
        sqlx::raw_sql(&trigger_sql(t))
            .execute(pool)
            .await
            .with_context(|| format!("trigger on {}", t.table))?;
    }
    Ok(())
}

async fn fetch_rows(pool: &PgPool, t: &TableCfg) -> Result<Vec<Row>> {
    let cols: Vec<&String> = t.columns.keys().chain(&t.context_columns).collect();
    let select: Vec<String> = cols.iter().map(|c| format!("{}::text", ident(c))).collect();
    let sql = format!(
        "SELECT {}::text, {} FROM {}",
        ident(&t.primary_key),
        select.join(", "),
        ident(&t.table)
    );
    // ponytail: full scan each sync, fine to ~100k rows; past that, diff hashes in SQL.
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .with_context(|| format!("reading {}", t.table))?;
    Ok(rows
        .iter()
        .map(|r| Row {
            id: r.get::<Option<String>, _>(0).unwrap_or_default(),
            values: cols
                .iter()
                .enumerate()
                .filter_map(|(i, c)| Some(((*c).clone(), r.get::<Option<String>, _>(i + 1)?)))
                .collect(),
        })
        .collect())
}

async fn fetch_shadow(pool: &PgPool, table: &str) -> Result<Shadow> {
    let rows = sqlx::query(
        r#"SELECT row_id, "column", locale, source_hash, status, value FROM glot_translations WHERE "table" = $1"#,
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .context("reading glot_translations (run `glot db install` first)")?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                (r.get(0), r.get(1), r.get(2)),
                Existing {
                    source_hash: r.get(3),
                    status: r.get(4),
                    value: r.get(5),
                },
            )
        })
        .collect())
}

/// Upsert one chunk's results in a transaction (an interrupted sync keeps every finished chunk).
async fn write(
    pool: &PgPool,
    table: &str,
    locale: &str,
    todo: &[Todo],
    results: &[translate::Translated],
) -> Result<(usize, usize)> {
    let mut tx = pool.begin().await?;
    let (mut ok, mut failed) = (0, 0);
    for t in todo {
        let Some(r) = results.iter().find(|r| r.key == t.item.key) else {
            continue;
        };
        let bad = validate::has_errors(&r.issues);
        if bad {
            failed += 1;
            tracing::warn!(key = t.item.key, issues = ?r.issues, "translation rejected by the validator");
        } else {
            ok += 1;
        }
        sqlx::query(
            r#"INSERT INTO glot_translations ("table", row_id, "column", locale, value, source_hash, status, model, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now())
               ON CONFLICT ("table", row_id, "column", locale) DO UPDATE SET value = EXCLUDED.value,
                 source_hash = EXCLUDED.source_hash, status = EXCLUDED.status, model = EXCLUDED.model, updated_at = now()"#,
        )
        .bind(table)
        .bind(&t.row_id)
        .bind(&t.column)
        .bind(locale)
        .bind(&r.value)
        .bind(&t.source_hash)
        .bind(if bad { "failed" } else { "translated" })
        .bind(&r.model)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok((ok, failed))
}

pub struct Ctx<'a> {
    pub cfg: &'a Config,
    pub db: &'a DbConfig,
    pub glossary: &'a Glossary,
    pub translator: &'a Translator,
    pub locales: &'a [String],
}

/// One pass over every table and locale. Returns the spend.
pub async fn sync_once(pool: &PgPool, c: &Ctx<'_>, dry_run: bool) -> Result<Usage> {
    let mut usage = Usage::default();
    let mut estimated = 0.0;
    let chunk_size = c.cfg.llm.batch_size.max(1) * 4;
    for t in &c.db.tables {
        let rows = fetch_rows(pool, t).await?;
        let shadow = fetch_shadow(pool, &t.table).await?;
        for loc in c.locales {
            let todo = select(t, &rows, loc, &shadow);
            if todo.is_empty() {
                continue;
            }
            println!("{} → {loc}: {} cell(s) to translate", t.table, todo.len());
            for chunk in todo.chunks(chunk_size) {
                let items: Vec<Item> = chunk.iter().map(|t| t.item.clone()).collect();
                let mut glossary = vec![];
                for i in &items {
                    for term in c.glossary.matches(&i.source) {
                        if !glossary
                            .iter()
                            .any(|g: &glot_core::glossary::Term| g.term == term.term)
                        {
                            glossary.push(term.clone());
                        }
                    }
                }
                let req = Request {
                    source_locale: c.cfg.source_locale.clone(),
                    target_locale: loc.clone(),
                    items,
                    glossary,
                    syntax: c.cfg.syntax,
                    style: c.cfg.llm.style.clone(),
                };
                let est = translate::estimate(c.cfg, &req).cost_usd;
                if dry_run {
                    estimated += est;
                    continue;
                }
                if let Some(b) = c.cfg.llm.budget_usd.filter(|b| usage.cost_usd + est > *b) {
                    bail!(
                        "budget: spent ${:.4}, next chunk ~${est:.4} > budget ${b:.2}; finished chunks are saved, rerun to resume",
                        usage.cost_usd
                    );
                }
                let (results, u) = c.translator.translate(req).await?;
                usage.add(&u);
                let (ok, failed) = write(pool, &t.table, loc, chunk, &results).await?;
                println!("  wrote {ok}, rejected {failed}");
            }
        }
    }
    if dry_run {
        println!("estimated cost: ${estimated:.4}");
    } else {
        println!("cost: ${:.4}", usage.cost_usd);
    }
    Ok(usage)
}

pub async fn run(a: DbArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = Config::find_root(&cwd).with_context(|| format!("no {CONFIG_FILE} found"))?;
    let (cfg, db) = load(&root)?;
    match a.cmd {
        DbCmd::Install { database_url } => {
            let pool = connect(database_url).await?;
            install(&pool, &db).await?;
            println!(
                "glot_translations + triggers installed on {} table(s)",
                db.tables.len()
            );
        }
        DbCmd::Sync {
            database_url,
            locales,
            dry_run,
            watch,
        } => {
            if let Some(l) = locales.iter().find(|l| !cfg.target_locales.contains(l)) {
                bail!("locale \"{l}\" is not in target_locales");
            }
            let locales = if locales.is_empty() {
                cfg.target_locales.clone()
            } else {
                locales
            };
            let pool = connect(database_url).await?;
            let glossary = Glossary::load(&root.join(GLOSSARY_FILE))?;
            let translator = Translator::new(&cfg, provider::from_config(&cfg.llm, None)?);
            let ctx = Ctx {
                cfg: &cfg,
                db: &db,
                glossary: &glossary,
                translator: &translator,
                locales: &locales,
            };
            if !watch {
                sync_once(&pool, &ctx, dry_run).await?;
                return Ok(ExitCode::SUCCESS);
            }
            let mut listener = sqlx::postgres::PgListener::connect_with(&pool).await?;
            listener.listen("glot_db").await?;
            let mut spent = 0.0;
            loop {
                spent += sync_once(&pool, &ctx, dry_run).await?.cost_usd;
                if let Some(b) = cfg.llm.budget_usd.filter(|b| spent >= *b) {
                    bail!("budget ${b:.2} spent (${spent:.4}) in this watch session");
                }
                println!("watching for changes (NOTIFY glot_db)…");
                listener.recv().await?;
                // Debounce bursts (bulk imports): drain what arrived meanwhile, then one sync.
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                while listener.next_buffered().is_some() {}
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> TableCfg {
        toml::from_str::<DbConfig>(
            r#"[[tables]]
table = "Product"
context = "Outdoor gear shop"
context_columns = ["category"]
columns = { name = { context = "Product name", max_length = 60 }, description = "Product page body" }
"#,
        )
        .unwrap()
        .tables
        .remove(0)
    }

    fn row(id: &str, name: &str, desc: &str) -> Row {
        Row {
            id: id.into(),
            values: [("name", name), ("description", desc), ("category", "Tents")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn done(shadow: &mut Shadow, todo: &[Todo], loc: &str, status: &str) {
        for t in todo {
            shadow.insert(
                (t.row_id.clone(), t.column.clone(), loc.into()),
                Existing {
                    source_hash: t.source_hash.clone(),
                    status: status.into(),
                    value: format!("[{loc}] {}", t.item.source),
                },
            );
        }
    }

    #[test]
    fn selects_missing_then_nothing_then_stale() {
        let t = table();
        let mut rows = vec![row("1", "Tent", "A light tent"), row("2", "Stove", "")];
        let mut shadow = Shadow::new();
        let todo = select(&t, &rows, "de", &shadow);
        // Empty description skipped.
        assert_eq!(todo.len(), 3);
        let name = todo
            .iter()
            .find(|t| t.row_id == "1" && t.column == "name")
            .unwrap();
        assert_eq!(name.item.context.max_length, Some(60));
        assert_eq!(name.source_hash, hash("Tent"));
        let d = name.item.context.description.as_deref().unwrap();
        assert!(
            d.contains("\"Product\"")
                && d.contains("Outdoor gear shop")
                && d.contains("Product name")
        );
        assert!(
            name.item
                .context
                .neighbors
                .contains(&("category".into(), "Tents".into()))
        );
        assert!(
            name.item
                .context
                .neighbors
                .contains(&("description".into(), "A light tent".into()))
        );

        // Idempotent: once written, a second pass selects nothing (other locales still pending).
        done(&mut shadow, &todo, "de", "translated");
        assert!(select(&t, &rows, "de", &shadow).is_empty());
        assert_eq!(select(&t, &rows, "fr", &shadow).len(), 3);

        // Source edit => hash differs => only that cell, with the previous translation as a hint.
        rows[0]
            .values
            .insert("description".into(), "An ultralight tent".into());
        let todo = select(&t, &rows, "de", &shadow);
        assert_eq!(todo.len(), 1);
        assert_eq!(todo[0].column, "description");
        assert_eq!(todo[0].item.previous.as_deref(), Some("[de] A light tent"));

        // Trigger-marked stale with the same hash is retranslated; failed with the same hash is not.
        let k = ("2".to_string(), "name".to_string(), "de".to_string());
        shadow.get_mut(&k).unwrap().status = "stale".into();
        assert_eq!(select(&t, &rows, "de", &shadow).len(), 2);
        shadow.get_mut(&k).unwrap().status = "failed".into();
        assert_eq!(select(&t, &rows, "de", &shadow).len(), 1);
    }

    #[test]
    fn hash_matches_postgres_expression() {
        // left(encode(sha256(convert_to('test','UTF8')),'hex'),16) in SCHEMA.
        assert_eq!(hash("test"), "9f86d081884c7d65");
    }

    #[test]
    fn migration_file_matches_schema() {
        let file = include_str!("../../../packages/db/migrations/0001_glot_translations.sql");
        assert_eq!(file, SCHEMA);
    }

    #[test]
    fn trigger_quotes_identifiers() {
        let mut t = table();
        t.table = "we\"ird".into();
        let sql = trigger_sql(&t);
        assert!(sql.contains("ON \"we\"\"ird\""));
        assert!(sql.contains("glot_db_changed('id', 'description', 'name')"));
    }

    /// End to end against Postgres: `GLOT_DB_TEST_URL=postgres://glot:glot@localhost:5433/postgres`.
    #[tokio::test]
    async fn sync_is_idempotent_and_resyncs_stale() -> Result<()> {
        let Ok(admin) = std::env::var("GLOT_DB_TEST_URL") else {
            return Ok(());
        };
        let name = format!("glot_dbl10n_test_{}", std::process::id());
        let admin = PgPool::connect(&admin).await?;
        sqlx::raw_sql(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await?;
        let url = format!(
            "{}/{name}",
            std::env::var("GLOT_DB_TEST_URL")?
                .rsplit_once('/')
                .unwrap()
                .0
        );
        let res = async {
            let pool = PgPool::connect(&url).await?;
            sqlx::raw_sql(
                r#"CREATE TABLE "Product" (id serial PRIMARY KEY, name text NOT NULL, description text, category text);
                   INSERT INTO "Product" (name, description, category) VALUES ('Tent', 'A light tent', 'Camping'), ('Stove', 'Boils water', 'Kitchen');"#,
            )
            .execute(&pool)
            .await?;
            let db = DbConfig { tables: vec![table()] };
            install(&pool, &db).await?;
            install(&pool, &db).await?; // idempotent
            let mut cfg: Config = toml::from_str(
                "source_locale = \"en\"\ntarget_locales = [\"de\", \"fr\"]\nfiles = []\n[llm]\nprovider = \"mock\"\nbudget_usd = 1.0\n",
            )?;
            cfg.llm.fallback_model = None;
            let glossary = Glossary::default();
            let translator = Translator::new(&cfg, provider::from_config(&cfg.llm, None)?);
            let locales = vec!["de".to_string(), "fr".to_string()];
            let ctx = Ctx { cfg: &cfg, db: &db, glossary: &glossary, translator: &translator, locales: &locales };
            let count = |sql: &'static str| {
                let pool = pool.clone();
                async move { sqlx::query_scalar::<_, i64>(sql).fetch_one(&pool).await }
            };

            sync_once(&pool, &ctx, false).await?;
            assert_eq!(count("SELECT count(*) FROM glot_translations WHERE status = 'translated'").await?, 8);
            let v: String = sqlx::query_scalar(
                r#"SELECT value FROM glot_translations WHERE row_id = '1' AND "column" = 'name' AND locale = 'de'"#,
            )
            .fetch_one(&pool)
            .await?;
            assert_eq!(v, "⟦de⟧ Tent");
            let before = count_ts(&pool).await?;
            sync_once(&pool, &ctx, false).await?;
            assert_eq!(count_ts(&pool).await?, before, "second sync must write nothing");

            // Source edit -> trigger marks stale; a no-op update of another column does not.
            sqlx::raw_sql(r#"UPDATE "Product" SET description = 'An ultralight tent' WHERE id = 1; UPDATE "Product" SET category = 'Outdoor' WHERE id = 2;"#)
                .execute(&pool)
                .await?;
            assert_eq!(count("SELECT count(*) FROM glot_translations WHERE status = 'stale'").await?, 2);
            sync_once(&pool, &ctx, false).await?;
            assert_eq!(count("SELECT count(*) FROM glot_translations WHERE status = 'stale'").await?, 0);
            let v: String = sqlx::query_scalar(
                r#"SELECT value FROM glot_translations WHERE row_id = '1' AND "column" = 'description' AND locale = 'fr'"#,
            )
            .fetch_one(&pool)
            .await?;
            assert_eq!(v, "⟦fr⟧ An ultralight tent");

            // Delete drops the row's translations.
            sqlx::raw_sql(r#"DELETE FROM "Product" WHERE id = 2"#).execute(&pool).await?;
            assert_eq!(count("SELECT count(*) FROM glot_translations").await?, 4);
            pool.close().await;
            anyhow::Ok(())
        }
        .await;
        sqlx::raw_sql(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
            .execute(&admin)
            .await?;
        res
    }

    async fn count_ts(pool: &PgPool) -> Result<String> {
        Ok(
            sqlx::query_scalar("SELECT coalesce(max(updated_at)::text, '') FROM glot_translations")
                .fetch_one(pool)
                .await?,
        )
    }
}
