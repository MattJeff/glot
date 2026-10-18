# Segmented copy experiments — CONTRAT (v1, 2026-10-18)

> Source de vérité inter-agents (XP1…XP10). Conception : `segmented-copy-experiments.md`.
> Toute modification après publication va dans **Changelog** (bas de fichier). Propriétaire : XP1.

## 0. Décisions qui s'écartent de la conception

- **Tables neuves préfixées `xp_`** au lieu d'`ALTER variant_experiments` : l'état est par cellule (looks, gagnant,
  poids), les statuts diffèrent (`draft`, `paused`), et le chemin legacy (`variants.rs`, `/api/projects/{slug}/experiments`,
  `variant_arms`) reste **intact**. Une seule expérience live par clé, tous systèmes confondus (cf. §1 `xp_keys.live` +
  contrôle croisé legacy ↔ xp).
- **API sous `/api/projects/{slug}/xp`** (`/experiments` est déjà pris par le legacy).
- Holdout = ligne `xp_experiments` `kind='holdout'` (une par projet), bras 0 = holdout, bras 1 = reste du trafic.
- Bras 0 = catalogue officiel : **jamais stocké** dans `xp_values` ; l'edge n'applique aucun overlay pour le bras 0.

## 1. SQL — `crates/server/migrations/0110_segmented_experiments.sql`

```sql
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
```

## 2. Types Rust publics — `crates/server/src/experiments/model.rs`

Re-exportés par `crate::experiments::*`. Tous `Serialize + Deserialize` (JSON = §4).

```rust
pub const MAX_CELLS: usize = 4;          // segments ciblés (+ '*')
pub const MAX_ARMS: i32 = 8;             // bras 0..=7
pub const ANY: &str = "*";               // cellule « reste du trafic »

pub enum Status { Draft, Running, Paused, Stopped, Promoted, Personalized }   // serde snake_case
pub enum Allocation { Fixed, Bandit, Segmented }                                // serde snake_case
pub enum Metric { Conversion, Revenue }                                         // serde snake_case

pub struct Predicate { pub attr: String, pub eq: Option<String>, pub r#in: Option<Vec<String>>,
                       pub not_in: Option<Vec<String>> }                         // exactement un opérateur
pub struct SegmentRule { pub all: Vec<Predicate> }                              // conjonction, 1..=8 prédicats
pub struct Segment { pub id: Uuid, pub name: String, pub rule: SegmentRule }
impl SegmentRule { pub fn validate(&self) -> Result<(), String>;
                   pub fn matches(&self, ctx: &BTreeMap<String, String>) -> bool; } // référence pour l'edge (tests croisés)

pub struct KeyGroup { pub keys: Vec<String>, pub prefix: Option<String> }       // entrée : keys OU prefix (« home.hero »)
pub struct Variant { pub arm: i32, pub intent: String, pub approved: bool,
                     pub values: BTreeMap<String /*locale*/, BTreeMap<String /*key*/, String>> }
pub type Intention = Variant;            // alias : un bras = une intention
pub struct Allocations { pub cell: String, pub weights: Vec<f64>, pub looks: i32,
                         pub winner: Option<i32>, pub personalized: bool }       // = ligne xp_cells
pub struct Experiment { /* JSON §4.2 */ }
pub struct Exposure { pub id: Uuid, pub cell: String, pub arm: i32 }            // élément de Hit.exposures (§6)
pub struct Conversion { pub event: String, pub value: Option<f64>, pub currency: Option<String>,
                        pub exposures: Vec<Exposure> }                           // vue logique d'un Hit avec event
pub struct CellCheck { pub arm: i32, pub key: String, pub locale: String, pub value: Option<String>,
                       pub ok: bool, pub issues: Vec<glot_core::validate::Issue> }
pub struct Readiness { pub ready: bool, pub problems: Vec<String>, pub cells: Vec<CellCheck> }
```

Fonctions publiques (XP1, `experiments/mod.rs`) :
```rust
pub fn routes() -> Router<AppState>;                                   // mergé dans lib.rs
pub async fn load(state: &AppState, project_id: Uuid, id: Uuid) -> ApiResult<Experiment>;
pub async fn readiness(state: &AppState, p: &ProjectRow, id: Uuid) -> ApiResult<Readiness>;
pub async fn live_key_owner(db: &PgPool, project_id: Uuid, key: &str) -> ApiResult<Option<Uuid>>; // pour le legacy
```

## 3. Segments (`SegmentRule`)

```json
{"all": [{"attr": "country", "in": ["DE", "AT", "CH"]}, {"attr": "source", "eq": "search"}]}
```
| attr | valeurs admises |
|---|---|
| `country` | ISO 3166-1 alpha-2 MAJUSCULE |
| `region` | code région (≤ 8 car. `[A-Z0-9-]`) |
| `locale` | locale BCP 47 (`validate_locale`) |
| `device` | `mobile` `tablet` `desktop` |
| `source` | `search` `social` `email` `paid` `direct` `referral` |
| `visitor` | `new` `returning` `unknown` |
| `plan` | chaîne libre ≤ 64 |
| `logged_in` | `true` `false` |
| `custom.<nom>` | nom `[a-z0-9_]{1,32}`, chaîne ≤ 64 |
Opérateurs : un seul parmi `eq` / `in` / `not_in` (listes 1..=20). Attribut absent du contexte ⇒ `eq`/`in` faux,
`not_in` vrai. Ordre des cellules = priorité, premier qui matche, sinon `*`.

## 4. REST (JSON, cookie de session ou `Authorization: Bearer <GLOT_TOKEN>`)

Préfixe `P = /api/projects/{slug}/xp`. Erreurs : enveloppe habituelle `{"error":{"code","message"}}`.
Rôles : **T** = translator (lecture + édition des textes en draft), **A** = admin, **O** = owner.

| Méthode | Route | Rôle | Corps → Réponse |
|---|---|---|---|
| GET | `P` | T | → `Experiment[]` (hors holdout, 100 max, live d'abord) |
| POST | `P` | A | `NewExperiment` → 201 `Experiment` (draft) |
| GET | `P/{id}` | T | → `Experiment` |
| PATCH | `P/{id}` | A | `NewExperiment` partiel → `Experiment` (draft seulement, sauf `name`) ; 409 sinon |
| DELETE | `P/{id}` | A | → 204 (draft ou stopped) |
| PUT | `P/{id}/arms/{arm}` | T | `{intent?, approved?, values?: {locale:{key:value}}}` → `Variant` (arm ≥ 1, draft ; `approved` exige A ; valeurs fusionnées, `""` supprime la cellule) |
| DELETE | `P/{id}/arms/{arm}` | A | → 204 (draft, arm ≥ 1) |
| GET | `P/{id}/check` | T | → `Readiness` |
| POST | `P/{id}/start` | A | → `Experiment` ; 422 `not_ready` (+ `readiness`) ; 409 clé déjà live ; 402 quota |
| POST | `P/{id}/pause` | A | running → paused |
| POST | `P/{id}/resume` | A | paused → running (re-valide comme start) |
| POST | `P/{id}/stop` | A | running/paused/personalized → stopped (libère les clés) |
| POST | `P/{id}/promote` | A | `{scope:"all"\|"segment", arm, cell?}` → `Experiment` (§4.3) |
| GET | `P/{id}/results` | T | **XP2** (`experiments/stats.rs`) |
| POST | `P/{id}/generate` | A | **XP8** (`{intents:[…], locales?}` → bras + `rejected`) |
| POST | `P/estimate` | T | **XP2** (trafic/durée d'un segment) |
| GET | `P/segments` | T | → `Segment[]` |
| POST | `P/segments` | A | `{name, rule}` → 201 `Segment` (409 nom pris) |
| PATCH | `P/segments/{id}` | A | `{name?, rule?}` → `Segment` (409 si utilisé par une expérience live) |
| DELETE | `P/segments/{id}` | A | → 204 (409 si utilisé par une expérience non-draft) |
| GET | `P/settings` | T | → `{holdout_pct, persistence, holdout_id}` |
| PUT | `P/settings` | O | `{holdout_pct?: 0..20, persistence?: "none"\|"session"}` → idem |

### 4.1 `NewExperiment`
```json
{"name": "Hero DACH", "keys": ["home.hero.title", "home.hero.cta"], "prefix": null,
 "locales": ["en", "de"], "cells": ["<segment uuid>", {"name": "FR mobile", "rule": {"all": [...]}}],
 "allocation": "segmented", "weights": null, "traffic_pct": 100, "min_n": 500,
 "goal": "signup", "metric": "conversion", "currency": null, "guardrails": ["refund"],
 "facts": ["2 000 équipes"], "fits": {"home.hero.title": {"max_width_px": 480, "max_lines": 2, "font": {...}}},
 "review": false,
 "arms": [{"intent": "social proof", "values": {"de": {"home.hero.title": "…"}}}]}
```
`keys` ou `prefix` (toutes les clés `prefix.*`, ≤ 20 clés). `locales` défaut = source + cibles. `cells` : uuid
existant ou segment inline (créé). `arms` → bras 1.. (bras 0 implicite). Défauts = colonnes SQL.

### 4.2 `Experiment`
```json
{"id": "uuid", "name": "Hero DACH", "kind": "copy", "status": "draft",
 "keys": [{"key": "home.hero.title", "source": "…", "source_changed": false}],
 "locales": ["en", "de"],
 "cells": [{"id": "uuid", "name": "DACH search", "rule": {...}}],
 "allocation": "segmented", "weights": null, "traffic_pct": 100, "min_n": 500,
 "goal": "signup", "metric": "conversion", "currency": null, "guardrails": [], "facts": [], "fits": {},
 "review": false,
 "arms": [{"arm": 0, "intent": "official", "approved": true, "values": {}},
          {"arm": 1, "intent": "social proof", "approved": true, "values": {"de": {"home.hero.title": "…"}}}],
 "state": [{"cell": "uuid", "weights": [0.5, 0.5], "looks": 0, "winner": null, "personalized": false},
           {"cell": "*", "weights": [0.5, 0.5], "looks": 0, "winner": null, "personalized": false}],
 "created_at": "…", "started_at": null, "ended_at": null}
```
`state` vide tant que draft. `source_changed` compare à `keys.source` actuel (draft : toujours false).

### 4.3 Activation & promotion
- **start/resume** : `Readiness.ready` exigé. Pour chaque bras ≥ 1 **approuvé**, chaque (clé, locale) : valeur non
  vide, `keys::check(source, value, locale, glossaire)` sans erreur, `measure::check(value, Icu, fits[key])` si fit.
  `problems` : < 1 bras approuvé, clé inexistante, clé live ailleurs (xp ou legacy `variant_experiments`), > 4 cellules,
  revenue sans currency (quota : 402 séparé, cf. Changelog v1.1). Bras non approuvés ignorés (ni servis ni comptés).
  Au start : `xp_keys.source` = source courante, `live=true`, `xp_cells` créées pour chaque cellule + `*`
  (poids = `weights` normalisés si `fixed`, sinon uniformes sur les bras approuvés — bras non approuvés poids 0).
- **promote `all`** : écrit le bras `arm` pour chaque (clé, locale cible ≠ source) via `keys::write_translation`
  (reviewed, historique, model `experiment`) ; statut `promoted`, clés libérées. Bras 0 = clôture sans écriture.
- **promote `segment`** : `xp_cells(cell).winner = arm, personalized = true, weights = one-hot` ; statut `personalized`
  (les autres cellules continuent d'être servies, poids figés, non comptées).

### 4.4 Quotas (billing, cloud seulement)
`billing::Limits` gagne `experiments` (expériences live simultanées) et `xp_cells` (segments ciblés par expérience,
0 = pas de segmentation) : free (1, 0), pro (5, 2), team (illimité, 4). Dépassement → 402 `plan_required` /
`quota_exceeded` au start/resume. Self-hosted : aucune limite (sauf `MAX_CELLS`).

## 5. Config publiée — `GET /cdn/{slug}/experiments.json` (XP3)

Mêmes règles que `cdn::serve` (public/privé, `billing::CDN`, ETag, `public, max-age=60, stale-while-revalidate=600`).
Route statique enregistrée **avant** `/cdn/{slug}/{file}` (axum : le statique gagne). Seules les expériences
`running` / `personalized` dont aucune clé n'a vu sa source changer ; seuls les bras approuvés.
```json
{"v": 1, "project": "acme", "generated_at": "2026-10-18T12:00:00Z", "max_stale": 86400,
 "persistence": "none",
 "holdout": {"id": "uuid", "pct": 5},
 "segments": [{"id": "uuid", "rule": {"all": [{"attr": "country", "in": ["DE"]}]}}],
 "experiments": [
   {"id": "uuid", "status": "running", "traffic_pct": 100, "goal": "signup", "guardrails": ["refund"],
    "metric": "conversion", "currency": null,
    "keys": ["home.hero.title", "home.hero.cta"], "locales": ["en", "de"],
    "cells": [{"id": "<segment uuid>", "weights": [0.2, 0.8], "count": true},
              {"id": "*", "weights": [0.5, 0.5], "count": true}],
    "arms": [{"arm": 0},
             {"arm": 1, "values": {"de": {"home.hero.title": "…", "home.hero.cta": "…"}}}]}]}
```
- `holdout` : `null` si `pct = 0`. Tirage holdout d'abord ; en holdout ⇒ aucune expérience.
- `cells[].weights` indexés comme `arms` ; `count:false` = cellule personnalisée (servir, ne pas compter).
- Locale servie absente de `locales` ⇒ expérience ignorée (bras 0, rien compté). Valeur manquante pour une clé ⇒ catalogue.
- Config de plus de `max_stale` secondes ⇒ bras 0 partout, rien compté.

## 6. Événements — `POST /api/insights/{pk}` (XP3 côté serveur, XP4/XP5 émetteurs)

`Hit` gagne trois champs optionnels :
```json
{"served": "de", "country": "DE",
 "exposures": [{"id": "<xp uuid>", "cell": "<segment uuid>|*", "arm": 1},
               {"id": "<holdout uuid>", "cell": "*", "arm": 1}],
 "event": "purchase", "value": 49.0, "currency": "EUR"}
```
- **Exposition** = Hit sans `event` avec `exposures` : +1 `xp_counts(event='$exposure')` par élément (seulement si
  l'expérience est running, la cellule `count`, le bras approuvé). Envoyée par l'edge (`waitUntil`) ou le runtime client,
  **une fois par document**. Jamais pour un bot (UA), opt-out, GPC/DNT, ou forçage `?glot_xp=`.
- **Conversion** = Hit avec `event` + les `exposures` de la session (≤ 20) : +1 `n` (+ `value`/`value²` si
  `currency` = celle de l'expérience) pour `goal` et `guardrails`. Le client crédite un `goal` **une fois par
  session et par expérience**. `value` ∈ [0, 1e6].
- Holdout : bras 0 = en holdout, bras 1 = hors holdout ; compté comme une expérience (`goal` = `signup` par défaut).
- Aucun autre attribut (pays de segment, UA, route) n'est stocké : seule la cellule matchée remonte.

## 7. En-tête `x-glot-xp` (edge → app, XP4 → XP5) et cookie

- Requête edge → origine : `x-glot-xp: <base64url(JSON)>` sans padding,
  `{"v":1,"locale":"de","a":[{"id":"…","cell":"*","arm":1}],"o":{"home.hero.title":"…"}}`
  (`a` = assignations, `o` = overlay clé→valeur pour la locale servie, bras 0 exclus). L'edge **supprime** tout
  `x-glot-xp` entrant du client. Absent/illisible ⇒ messages officiels.
- Cookie (seulement si `persistence = "session"`) : `glot_xp=<id>.<arm>.<cell>~<id>.<arm>.<cell>` (session, `Path=/`,
  `SameSite=Lax`, `Secure`, pas `HttpOnly` pour que le runtime le lise). Assignation dont le bras n'est plus servi ⇒
  re-tirage. Holdout inclus (`<holdout id>.0.*`).
- Forçage aperçu : `?glot_xp=<id>.<arm>` (non compté, prioritaire).
- Le runtime client recopie `a` dans sessionStorage (`glot_xp`) pour créditer les conversions.

## 8. Rétrocompatibilité
Legacy (`variants.rs`, une clé × une locale) inchangé ; `variants::create` refuse (409) une clé live dans `xp_keys`,
et `start` refuse une clé live dans `variant_experiments`.

## Changelog
- 2026-10-18 v1 — publication initiale.
- 2026-10-18 v1.1 (XP1) —
  - `xp_cells.weights[i]` = poids du bras **numéro** i (bras non approuvés / supprimés = 0). La config CDN (§5) ne publie
    que les bras approuvés : XP3 filtre `weights` en conséquence (indices alignés sur `arms` de la config).
  - Quota du plan : **pas** dans `Readiness.problems`, uniquement 402 au start/resume (`billing::check_experiment`).
  - `POST P` accepte aussi `review` : si vrai, les bras passés à la création sont `approved=false`.
  - Settings holdout : `PUT P/settings` crée/maj la pseudo-expérience holdout (`xp_cells('*').weights = [pct, 1-pct]`,
    status `running` si pct > 0, sinon `paused`).
- 2026-10-18 v1.2 (XP2) —
  - Migration `0111_xp_analyses.sql` : `xp_analyses(experiment_id uuid FK xp_experiments ON DELETE CASCADE,
    at timestamptz DEFAULT now(), look int, decision boolean, paused boolean, analysis jsonb, PK(experiment_id, at))`.
  - Handlers `experiments::rebalance::{results, estimate}` montés par XP1 sur `GET P/{id}/results` et `POST P/estimate`.
  - `GET P/{id}/results` → `{"latest": Analysis|null, "history": [{"at","look","decision","paused","analysis"}]}`
    (plus récent d'abord, ≤ 50). `Analysis` (= `stats::Analysis`) :
    `{look, decision_look, threshold, global: CellResult, cells: [CellResult], holdout_srm_p, paused,
      everywhere: arm|null, segments: [[cell, arm]]}` ;
    `CellResult = {name (id de cellule ou "*"), views, arms: [{mean, p_best, p_beats_control, lift, lift_lo, lift_hi,
      weight}], winner, p_specific, srm_p}` — indices de `arms` = numéros de bras (non approuvés présents, weight 0).
  - Worker rebalance : écrit `xp_cells.weights/looks/winner/decided_at` (poids au plus 1×/jour UTC, décisions à chaque
    tick) ; SRM p < 0,001 ⇒ `xp_experiments.status = 'paused'` (les clés restent réservées).
- 2026-10-18 v1.3 (XP3) —
  - `Hit` gagne aussi `dedup` (optionnel) : id aléatoire **par beacon** (8-64 car. `[A-Za-z0-9_-]`), réutilisé à
    l'identique en cas de retry ⇒ compté une fois (table UNLOGGED `xp_dedup`, migration `0115_xp_ingest.sql`, 1 h).
  - **Lot edge** : `POST /api/insights/{pk}/batch`, corps = tableau JSON de `{served, exposures, event?, value?,
    currency?, dedup?}` (≤ 100, 64 Ko) → `{"accepted","duplicates"}`. Expériences seulement (aucun `$view`
    `insight_daily`). Chaque élément = 1 hit du rate limit partagé et du quota `insights_events`. CORS `*`.
    Un `Hit` simple avec `exposures` (client) compte toujours le `$view` legacy **et** les expositions.
  - Serveur : UA de bot connu (`experiments::ingest::is_bot`) ⇒ 200 identique, rien compté. Une expo/conv n'est
    comptée que si : expérience `running` du projet, `served` ∈ `locales`, cellule non personnalisée, bras approuvé
    (holdout : cellule `*`, bras 0/1) ; une seule expo par expérience et par beacon.
  - Revenu : `value` converti vers `currency` de l'expérience (table statique ~24 devises) au lieu d'être ignoré si
    la devise diffère ; devise inconnue ⇒ conversion comptée, revenu 0.
  - Config §5 : l'ETag ignore `generated_at` (304 si inchangé) ; l'edge compte `max_stale` depuis son dernier 200/304.
    `holdout` publié seulement si pct > 0 et pseudo-expérience `running`.
- 2026-10-18 demande XP10 → XP2 (non bloquant pour les autres) —
  - Le worker rebalance ticke toutes les 300 s (1er tick au boot) : `GET P/{id}/results` reste `latest: null` pendant
    un e2e. Demande : `GLOT_XP_TICK_SECS` (défaut 300) lu dans `spawn_worker`. L'e2e `examples/experiments-demo`
    démarre glot-server avec `GLOT_XP_TICK_SECS=2` (sans effet tant que non implémenté).
- 2026-10-18 v1.4 (XP8) — génération, policy, personnalisation (`experiments/generate.rs`, monté dans `lib.rs`) :
  - `POST P/{id}/generate` (A, draft seulement, 409 sinon) : `{intents: [..], locales?, page?, segment?}` ;
    intents = `social_proof direct_benefit honest_urgency simplicity curiosity authority reassurance segment`
    (`"social proof"` accepté, 400 si inconnue). → `{arms: Variant[], rejected: [{intent, rationale, cells:
    [{key, locale, value, issues, findings}]}], warnings: [{intent, key, locale, value, rule, severity, message}],
    cost_usd}`. Bras stockés avec `intent` lisible (`"social proof"`), `approved = !review` ; régénérer une intention
    remplace son bras (mêmes numéro, valeurs des `locales` demandées). Un bras n'est écrit que si **toutes** les
    (clé, locale) passent validateur + glossaire + `fits` + policy. `segment` défaut = noms/règles des cellules,
    `page` défaut = nom de l'expérience. Erreur LLM → 422 `llm_error`.
  - Policy projet : migration `0117_copy_policy.sql` (`projects.copy_policy jsonb DEFAULT '{}'`),
    `GET P/policy` (T) / `PUT P/policy` (A) = `glot_core::policy::PolicyConfig`
    `{rules: {rule: "off"|"warning"|"error"}, forbidden: {"*"|lang|locale: [..]}, patterns: {rule: [..]}}` ;
    règles : `fake_urgency fake_counter unprovable_claim superlative unsourced_number price_or_legal confirmshaming
    forbidden_term`. Même table `[policy]` lue dans glot.toml par `PolicyConfig::from_toml`.
  - `POST P/{id}/personalize` (A) `{arm, cell}` : draft → `personalized` **sans test** : cellule `cell` one-hot sur
    `arm`, autres cellules + `*` one-hot bras 0, toutes `personalized=true` (rien compté). Mêmes contrôles que start
    (readiness + policy, clés live, `billing::check_experiment`). Fin via `stop`.
  - **Demande XP1** : `generate::policy_problems(state, p, &experiment) -> Vec<String>` à ajouter dans
    `readiness().problems` pour que `start` refuse aussi un texte édité à la main qui viole la policy (aujourd'hui
    seul `personalize` l'applique).
- 2026-10-18 constats e2e XP10 (`examples/experiments-demo`, `node --test test/e2e.test.ts`) —
  - **→ XP2** : `xp_analyses` n'est écrit que si `publish` (1×/jour UTC) ou look de décision/pause/décision ⇒ après
    le 1er tick du jour, `GET P/{id}/results` garde `latest.views = 0` jusqu'au lendemain alors que `xp_counts` a
    12 expositions + 8 conversions (DE). Demande : persister un snapshot à chaque tick (ou au moins quand les compteurs
    ont changé), en gardant `weights` NULL hors publication ; ou calculer `latest` à la volée dans `results`.
  - **→ XP4** : 12 visiteurs DE (1 document `/de` chacun + 1 navigation client vers `/de/signup`) ⇒ **68**
    `$exposure` au lieu de 12. Les requêtes RSC de Next 16 sont comptées : prefetch par segments
    (`next-router-segment-prefetch`, sans `next-router-prefetch`) et navigations client (`rsc: 1`). §6 dit « une fois par
    document » : ne compter que si `rsc` est absent (et `sec-fetch-dest: document` quand présent). L'assignation et
    l'overlay restent à faire sur ces requêtes (sinon le RSC rendrait l'officiel).
- 2026-10-18 v1.5 (XP2) — moteur stats (`experiments/stats.rs`, pur) + worker (`experiments/rebalance.rs`) :
  - `xp_analyses` a aussi `weights jsonb` (`{cell: [poids par numéro de bras]}`, non NULL seulement quand cette
    analyse a publié de nouveaux poids = historique rejoué par le SRM).
  - `Analysis` gagne `arm_ids` (bras servis) ; dans `results`, tous les `arms` / `winner` / `everywhere` /
    `segments` sont indexés par **numéro** de bras (trous = `ArmResult` vide, weight 0).
  - `GLOT_XP_TICK_SECS` implémenté (défaut 300, 1er tick au boot) ; `spawn_worker` lancé dans `main.rs`.
  - Poids publiés au plus 1×/jour UTC ; décisions vérifiées à chaque tick (un look n'est consommé que si décision
    possible : ≥ 7 j, trafic ≥ `next_look`, pas de SRM) ; SRM testé 1×/jour, p < 0,001 Bonferroni sur les cellules.
  - Top-two Thompson à 2 bras = 50/50 par construction (comme le legacy) : le bandit n'économise du trafic qu'à ≥ 3 bras.
  - `POST P/estimate` : `{rule?: SegmentRule, arms?: 2, goal?: "signup", min_n?: 500}` →
    `{daily_visitors, baseline, per_arm, days, too_long, exact}` (`exact=false` si la règle utilise autre chose que
    country/locale, que `insight_daily` n'enregistre pas ⇒ majorant ; `too_long` = > 90 j ⇒ fusionner dans `*`).
- 2026-10-18 v1.6 (XP5) — exposition « vue », runtime :
  - Migration `0130_xp_exposure.sql` : `xp_experiments.exposure text NOT NULL DEFAULT 'render'` (`render`|`seen`) ;
    `NewExperiment`/`Experiment` gagnent `exposure` (draft seulement). `seen` ⇒ `rebalance` prend `$seen` comme
    dénominateur (vues **et** SRM) ; `render` (défaut) = `$exposure` comme avant. Holdout : toujours `$exposure`.
  - `Exposure` gagne `seen?: bool`, `visible_ms?: number` (tranches 1000/2000/5000/10000/30000/60000, ≤ 60 000).
    Hit sans `event` avec `seen: true` ⇒ `xp_counts(event='$seen')` : `n` = sessions ayant vu le bloc (≥ 50 % visible
    ≥ 1 s, une fois par session et expérience), `value_sum` = Σ ms visibles en tranches. Envoyé par `@glot/insights`
    sur `POST /api/insights/{pk}/batch` (pas de `$view`), jamais sous GPC/DNT ; bots filtrés par l'ingest.
  - `results` : chaque `ArmResult` gagne `seen?: int` et `visible_ms?: float` (moyenne), dans `global` et `cells`.
  - **→ XP4** : en-tête `x-glot-xp` gagne `k` optionnel `{<xp id>: [clés]}` (= `experiments[].keys` de la config,
    à ajouter au type `XpConfig` de l'edge) : le runtime observe `[data-glot-key]` / `[data-glot]` de ces clés. Sans `k`,
    pas de « vue » (le mode `render` reste correct).
  - Runtime : si `x-glot-xp.locale` ≠ locale servie, l'assignation n'est ni stockée ni créditée (textes officiels).
