-- Analysis snapshots of segmented experiments (experiments/rebalance.rs). One row per analysis that changed
-- something: the day's new weights, a decision look, a winner, an SRM pause. `weights` ({cell: [w per arm]}) is
-- set only when that analysis published new weights: it is the history the SRM check replays.
CREATE TABLE xp_analyses (
  experiment_id uuid NOT NULL REFERENCES xp_experiments(id) ON DELETE CASCADE,
  at timestamptz NOT NULL DEFAULT now(),
  look int NOT NULL,
  decision boolean NOT NULL,
  paused boolean NOT NULL,
  weights jsonb,
  analysis jsonb NOT NULL,
  PRIMARY KEY (experiment_id, at));
