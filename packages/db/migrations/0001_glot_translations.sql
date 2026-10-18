CREATE TABLE IF NOT EXISTS glot_translations (
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
