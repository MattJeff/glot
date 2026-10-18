-- Segmented experiments: denominator of the stats. 'render' = documents served with the arm (edge `$exposure`),
-- 'seen' = the tested block was ≥ 50 % visible for ≥ 1 s (client `$seen`, once per session and experiment;
-- `xp_counts.value_sum` of `$seen` rows = Σ bucketed visible ms).
ALTER TABLE xp_experiments
  ADD exposure text NOT NULL DEFAULT 'render' CHECK (exposure IN ('render', 'seen'));
