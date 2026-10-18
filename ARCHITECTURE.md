# glot — architecture (contrat partagé)

> Nom de code `glot`. Open source (MIT). i18n de SaaS fiable avec l'IA :
> Git = source de vérité, l'agent de code = interface principale, le LLM est toujours encadré par du code déterministe.

## Principes
1. **Déterministe autour du LLM** : chaque traduction passe par `glot_core::validate` + glossaire. Échec → retry avec les issues dans le prompt, puis escalade vers `fallback_model`.
2. **Lockfile** (`glot.lock`) : hash source/target par (locale, key). On ne traduit que le delta. Une cible éditée à la main (target_hash ≠ hash courant) n'est **jamais** écrasée sans `--force`.
3. **BYO key** : l'utilisateur fournit sa clé LLM. Coût affiché et budgeté.
4. **Contexte** : extrait du code (rôle UI, composant, fichier:ligne, clés voisines) + glossaire + notes.
5. Compatible avec les libs existantes (next-intl, i18next, FormatJS…) : on ne fournit **pas** de runtime.

## Layout & propriété (1 agent = 1 zone, ne pas modifier les zones des autres)
| Chemin | Rôle |
|---|---|
| `crates/core` (`glot-core`) | Pur, sync, sans réseau : config, catalogues JSON, validateur ICU/i18next/MF2, pluriels CLDR, lockfile, glossaire, hash |
| `crates/extract` (`glot-extract`) | Analyse JS/TS/JSX (oxc) : usages de clés, chaînes en dur, rôle UI, clés inutilisées |
| `crates/llm` (`glot-llm`) | Providers (Anthropic, OpenAI, OpenAI-compatible/Ollama, mock), pipeline de traduction + retry, juge (score 0-100), routage/coût, `engine::Workspace` (opérations projet local) |
| `crates/mcp` (`glot-mcp`) | Serveur MCP stdio (rmcp) sur un `Workspace` local |
| `crates/cli` (`glot` bin) | CLI clap ; `glot mcp` appelle `glot_mcp::serve_stdio` |
| `crates/server` (`glot-server`) | API HTTP axum + Postgres (sqlx), jobs de traduction, CDN/OTA, SSE |
| `web/` | Dashboard Next.js (App Router, TS, Tailwind v4, shadcn/ui, next-intl) — **traduit par glot lui-même (dogfooding)** |
| `packages/visual-qa` | QA visuelle Playwright (débordements par locale) |
| `action/` | GitHub Action (`glot check` + `glot translate` + commentaire de PR) |

Ports dev : API `8790`, web `3100`, Postgres docker `5433` (`postgres://glot:glot@localhost:5433/glot`).

## Fichiers projet (côté utilisateur)
`glot.toml` :
```toml
source_locale = "en"
target_locales = ["fr", "es", "de", "ja", "ar"]
files = ["messages/{locale}.json"]   # {locale} remplacé
format = "auto"                      # auto | json-nested | json-flat
syntax = "icu"                       # icu | i18next | mf2

[llm]
provider = "anthropic"               # anthropic | openai | openai-compatible | mock
model = "claude-haiku-5-5"
fallback_model = "claude-sonnet-5-5"
judge_model = "claude-sonnet-5-5"    # optionnel
api_key_env = "ANTHROPIC_API_KEY"
# base_url = "http://localhost:11434/v1"
budget_usd = 5.0
style = "Friendly, concise, tutoiement en français"

[qa]
score_threshold = 80
max_length_ratio = 1.6

[extract]
include = ["src/**/*.{ts,tsx,js,jsx}", "app/**/*.{ts,tsx}"]

[server]                              # optionnel
url = "http://localhost:8790"
project = "my-saas"
token_env = "GLOT_TOKEN"
```

`glossary.toml` :
```toml
[[term]]
term = "Workspace"
description = "Espace partagé d'une équipe"
translations = { fr = "Espace de travail", de = "Arbeitsbereich" }
forbidden = { fr = ["Espace de travaux"] }

[[term]]
term = "glot"
do_not_translate = true
```

`glot.lock` (JSON trié, généré) :
```json
{ "version": 1,
  "notes": { "auth.login.cta": "Bouton principal de la page de connexion" },
  "entries": { "fr": { "auth.login.cta": {
      "source_hash": "9f86d081884c7d65", "target_hash": "2c26b46b68ffc68f",
      "origin": { "kind": "machine", "model": "claude-haiku-5-5" },
      "score": 92, "reviewed": false } } } }
```
Clés : chemin pointé (`auth.login.cta`).

## API HTTP (`glot-server`) — snake_case JSON, erreurs `{"error":{"code","message"}}`
Auth : cookie `glot_session` (web) ou `Authorization: Bearer <token>` (CLI/MCP).

```
POST   /api/auth/signup        {email,password,name}         -> User   (+cookie)
POST   /api/auth/login         {email,password}              -> User   (+cookie)
POST   /api/auth/logout
GET    /api/me                                               -> User {id,email,name}
GET    /api/tokens                                           -> [ApiToken {id,name,created_at,last_used_at}]
POST   /api/tokens             {name}                        -> {token, api_token: ApiToken}   (token montré une fois)
DELETE /api/tokens/{id}

GET    /api/projects                                         -> [Project]
POST   /api/projects           {name,slug,source_locale,target_locales}
GET    /api/projects/{slug}                                  -> Project
PATCH  /api/projects/{slug}    {name?,target_locales?,llm?: LlmSettings,style?,public_cdn?}
DELETE /api/projects/{slug}

Project = {id,slug,name,source_locale,target_locales[],style,public_cdn,created_at,
           llm: {provider,model,fallback_model,judge_model,has_api_key,budget_usd},
           stats: {keys, locales: {<loc>: {translated,reviewed,stale,issues}}}}
LlmSettings = {provider,model,fallback_model?,judge_model?,api_key?(write-only),base_url?,budget_usd?}

GET    /api/projects/{slug}/keys?q=&locale=&status=&page=1&per_page=50
         -> {items:[KeyRow], total}
KeyRow = {key, description, source, refs:[{path,line}], ui_role|null, component|null,
          translations: {<loc>: Translation}}
Translation = {value, status: "missing"|"machine"|"reviewed"|"stale",
               score|null, issues:[Issue], origin: "machine"|"human"|"imported", model|null, updated_at}
Issue = {code, severity:"error"|"warning", message}
POST   /api/projects/{slug}/keys                 {key,source,description?}
PATCH  /api/projects/{slug}/keys/{key}           {source?,description?}
DELETE /api/projects/{slug}/keys/{key}
PUT    /api/projects/{slug}/keys/{key}/translations/{locale}   {value} -> Translation (validé, origin=human, status=reviewed)
POST   /api/projects/{slug}/keys/{key}/translations/{locale}/review  -> Translation (status=reviewed)
GET    /api/projects/{slug}/keys/{key}/history  -> [{locale,value,origin,model,score,created_at,author}]

GET    /api/projects/{slug}/glossary             -> [Term {id,term,description,do_not_translate,case_sensitive,translations,forbidden}]
POST   /api/projects/{slug}/glossary             Term sans id
PATCH  /api/projects/{slug}/glossary/{id}
DELETE /api/projects/{slug}/glossary/{id}

GET    /api/projects/{slug}/issues               -> [{key,locale,issue:Issue}]

POST   /api/projects/{slug}/jobs   {kind:"translate", locales?, keys?, force?} -> Job
GET    /api/projects/{slug}/jobs                 -> [Job]
GET    /api/jobs/{id}                            -> Job
GET    /api/jobs/{id}/events                     (SSE: event "progress" data=Job, event "done")
Job = {id,kind,status:"queued"|"running"|"done"|"failed",progress:{done,total},cost_usd,error|null,created_at,finished_at|null}

POST   /api/projects/{slug}/sync/push  {catalogs:{<loc>:{<key>:value}}, lock, glossary:[Term sans id],
         context?:{<key>:{refs:[{path,line}], ui_role?, component?, description?}}} -> {created,updated,conflicts:[{locale,key,server,local}]}
         context = extraction du code (glot push) : remplace refs/ui_role/component des clés présentes, les autres gardent les leurs ;
         description = lock.notes sinon context.description, jamais effacée si absente (édition dashboard conservée).
         Les jobs serveur passent refs/ui_role/component/description au KeyContext du LLM.
GET    /api/projects/{slug}/sync/pull  -> {catalogs, lock (notes = descriptions serveur), glossary:[Term sans id]}

GET    /cdn/{slug}/{locale}.json       (OTA, public si public_cdn, ETag + Cache-Control) -> JSON imbriqué
GET    /api/openapi.json
GET    /healthz
```

## Review API (rétro-traduction, `crates/server/src/review.rs`)
« Faire confiance à une langue qu'on ne parle pas » : le LLM rétro-traduit littéralement la cible vers la langue source,
explique ses choix et donne un verdict de fidélité. Résultat mis en cache par (key, locale) et invalidé dès que la source
ou la cible change (hashes). Rôle minimal : translator.
```
POST   /api/projects/{slug}/review       {locale, keys?:[key], refresh?:bool} -> [Review]
         keys absent = toutes les clés traduites de la locale (max 200, les plus récentes d'abord) ; clés sans traduction ignorées.
         Réponses en cache renvoyées sans appel LLM sauf refresh=true. Erreurs : 400 locale inconnue, 422 si le provider LLM échoue (code "llm_error").
GET    /api/projects/{slug}/keys/{key}/review?locale=fr        -> Review  (calcule + met en cache si absent/périmé ; 404 si pas de traduction)
Review = {key, locale, source, target,
          back_translation,          // rétro-traduction littérale en source_locale
          verdict: "same"|"nuance"|"different",
          explanation,               // registre, termes du glossaire, pluriels, adaptations culturelles (langue source, 1-3 phrases)
          suggestion: string|null,   // meilleure cible proposée si verdict != "same"
          cached: bool, created_at}
```
UI : surligner `verdict="different"` (rouge) et `"nuance"` (orange) ; « Appliquer la suggestion » = `PUT …/translations/{locale}` {value: suggestion}.

CLI (local, cache `.glot/review.json`, pas de changement de glot.lock) :
`glot review [--locale fr] [--keys a,b] [--json] [--refresh] [--fix]` · `glot review --stale [--yes]` (garder = rafraîchir le hash du lock si la
cible reste fidèle à la nouvelle source, sinon retraduire) · `glot glossary mine [--apply] [--json]`.
MCP : `review_translation {locale?, keys?}`, `mine_glossary {apply?}`.

## Visual QA & locale matrix (`crates/server/src/visual.rs`, migration `0010_visual_reports.sql`)
```
POST   /api/projects/{slug}/visual-reports   {report: <visual-qa report.json>, files: {"<path du report>": "<base64>"}, label?}
         -> VisualReportMeta (201). Corps ≤ 128 Mo ; chemins relatifs .png/.jpg/.webp uniquement ; 20 derniers rapports conservés.
GET    /api/projects/{slug}/visual-reports                 -> [VisualReportMeta]   (plus récent d'abord)
GET    /api/projects/{slug}/visual-reports/latest          -> VisualReport          (404 si aucun)
GET    /api/projects/{slug}/visual-reports/{id}            -> VisualReport
GET    /api/projects/{slug}/visual-reports/{id}/files/{*path}  -> image (privée, immutable)
VisualReportMeta = {id, label, created_at, summary: {pages, issues, locales[], routes[], viewports[],
                    by_kind:{<kind>:n}, by_locale:{<loc>:n}, pages_by_locale:{<loc>:n}}}
VisualReport = VisualReportMeta + {report}
GET|PUT /api/projects/{slug}/preview   PreviewSettings {base_url|null, url_pattern ("/{locale}{route}"), routes[]}   (PUT : admin)
```
CLI : `glot-visual-qa … --upload <server-url> --project <slug> [--label x]` (token `GLOT_TOKEN`, `packages/visual-qa/src/upload.ts`).
Dashboard : `/app/{slug}/preview` (matrice de locales en iframes) et `/app/{slug}/visual` (galerie, avant/après).
Bridge optionnel côté app (`web/lib/preview-bridge.ts`, snippet affiché dans la matrice) : `postMessage` `glot:height` / `glot:ready` vers le parent, reçoit `glot:pseudo {on}`.
