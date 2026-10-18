-- Segmented copy experiments (docs/design/segmented-copy-experiments-contract.md): key groups × locales × segment
-- cells, per-cell bandit state, aggregate counters only (no visitor id). Legacy variant_experiments untouched.
ALTER TABLE projects
  ADD xp_holdout_pct int NOT NULL DEFAULT 0 CHECK (xp_holdout_pct BETWEEN 0 AND 20),
  ADD xp_persistence text NOT NULL DEFAULT 'none' CHECK (xp_persistence IN ('none', 'session'));

CREATE TABLE xp_segments (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name text NOT NULL,
  rule jsonb NOT NULL,                       -- SegmentRule (§3)
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE (project_id, name));

CREATE TABLE xp_experiments (
  id uuid PRIMARY KEY,
  project_id uuid NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind text NOT NULL DEFAULT 'copy' CHECK (kind IN ('copy', 'holdout')),
  name text NOT NULL,
  status text NOT NULL DEFAULT 'draft'
    CHECK (status IN ('draft', 'running', 'paused', 'stopped', 'promoted', 'personalized')),
  locales text[] NOT NULL DEFAULT '{}',      -- locales couvertes (peut inclure la source)
  cells uuid[] NOT NULL DEFAULT '{}',        -- segments ciblés, ordre = priorité, ≤ 4 ; '*' implicite
  allocation text NOT NULL DEFAULT 'bandit' CHECK (allocation IN ('fixed', 'bandit', 'segmented')),
  weights double precision[],                -- 'fixed' : un poids par bras (normalisé) ; NULL = uniforme
  traffic_pct int NOT NULL DEFAULT 100 CHECK (traffic_pct BETWEEN 1 AND 100),
  min_n int NOT NULL DEFAULT 500 CHECK (min_n BETWEEN 50 AND 1000000),
  goal text NOT NULL DEFAULT 'signup',       -- événement décisionnel
  metric text NOT NULL DEFAULT 'conversion' CHECK (metric IN ('conversion', 'revenue')),
  currency text,                             -- ISO 4217, requis si metric='revenue'
  guardrails text[] NOT NULL DEFAULT '{}',   -- événements qui ne doivent pas monter (ex. 'refund')
  facts jsonb NOT NULL DEFAULT '[]',         -- ["2 000 équipes", "note 4,8"] : seuls faits citables
  fits jsonb NOT NULL DEFAULT '{}',          -- {key: glot_core::measure::Fit} (.glot/layout.json)
  review boolean NOT NULL DEFAULT false,     -- bras générés en approved=false jusqu'à approbation
  created_at timestamptz NOT NULL DEFAULT now(),
  started_at timestamptz,                    -- 1er passage en running
  ended_at timestamptz);                     -- stopped | promoted
CREATE UNIQUE INDEX xp_experiments_holdout ON xp_experiments (project_id) WHERE kind = 'holdout';
CREATE INDEX xp_experiments_project ON xp_experiments (project_id, status);

-- Groupe de clés. `source` = texte source figé au démarrage (source changée => plus servi, comme le legacy).
-- `live` = vrai en running|paused|personalized : une clé n'est live que dans une expérience.
CREATE TABLE xp_keys (
  experiment_id uuid NOT NULL REFERENCES xp_experiments(id) ON DELETE CASCADE,
  project_id uuid NOT NULL,
  key text NOT NULL,
  source text NOT NULL DEFAULT '',
  live boolean NOT NULL DEFAULT false,
  PRIMARY KEY (experiment_id, key));
CREATE UNIQUE INDEX xp_keys_live ON xp_keys (project_id, key) WHERE live;

-- Bras = intention. Bras 0 = 'official' (créé avec l'expérience, sans valeurs).
CREATE TABLE xp_arms (
  experiment_id uuid NOT NULL REFERENCES xp_experiments(id) ON DELETE CASCADE,
  arm int NOT NULL CHECK (arm BETWEEN 0 AND 7),
  intent text NOT NULL,
  approved boolean NOT NULL DEFAULT true,
  PRIMARY KEY (experiment_id, arm));

CREATE TABLE xp_values (
  experiment_id uuid NOT NULL,
  arm int NOT NULL CHECK (arm >= 1),
  key text NOT NULL,
  locale text NOT NULL,
  value text NOT NULL,
  PRIMARY KEY (experiment_id, arm, key, locale),
  FOREIGN KEY (experiment_id, arm) REFERENCES xp_arms ON DELETE CASCADE);

-- État par cellule ('*' ou id de segment en texte). Créé au start (poids uniformes ou `weights`), mis à jour
-- à chaque look par le moteur stats (XP2). Lu par la config CDN (XP3).
CREATE TABLE xp_cells (
  experiment_id uuid NOT NULL REFERENCES xp_experiments(id) ON DELETE CASCADE,
  cell text NOT NULL,
  weights double precision[] NOT NULL,       -- une entrée par bras, somme 1
  looks int NOT NULL DEFAULT 0,
  winner int,                                -- gagnant déclaré (cellule)
  personalized boolean NOT NULL DEFAULT false, -- 100 % `winner`, non compté
  decided_at timestamptz,
  PRIMARY KEY (experiment_id, cell));

-- Compteurs agrégés uniquement. event='$exposure' => n = expositions ; sinon n = conversions de cet événement
-- (goal + guardrails), value_sum/value_sq = revenu dans `currency` de l'expérience.
CREATE TABLE xp_counts (
  experiment_id uuid NOT NULL REFERENCES xp_experiments(id) ON DELETE CASCADE,
  cell text NOT NULL,
  arm int NOT NULL,
  day date NOT NULL,
  event text NOT NULL,
  n bigint NOT NULL DEFAULT 0,
  value_sum double precision NOT NULL DEFAULT 0,
  value_sq double precision NOT NULL DEFAULT 0,
  PRIMARY KEY (experiment_id, cell, arm, day, event));
