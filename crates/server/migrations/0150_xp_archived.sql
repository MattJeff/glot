-- Archived experiments: hidden from the default dashboard list, results kept. Only archived (or draft)
-- experiments can be deleted, so stopping then deleting never drops results silently.
ALTER TABLE xp_experiments ADD archived_at timestamptz;
