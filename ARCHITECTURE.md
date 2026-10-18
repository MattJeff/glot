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
| `packages/agent-qa` | Parité de tâche (`glot parity`, `crates/cli/src/parity.rs`) : agent Claude vision ou mock scripté, `glot.tasks.yaml`, rapport local `.glot/parity/` (pas d'upload) |
| `action/` | GitHub Action (`glot check` + `glot translate` + commentaire de PR) |

Ports dev : API `8790`, web `3100`, Postgres docker `5433` (`postgres://glot:glot@localhost:5433/glot`).

## Coûts LLM
- **Prompt caching** : le system prompt (règles + glossaire de la locale, identique pour tous les lots d'une locale) est le
  préfixe stable, marqué `cache_control` (Anthropic) ; le glossaire n'est plus répété dans le payload utilisateur. OpenAI
  cache automatiquement le même préfixe. `Usage` compte `cache_creation_tokens` / `cache_read_tokens`, prix réels dans
  `pricing::cost_cached_usd` (écriture 1,25×, lecture 0,1× ; Opus 5.5 0,05× ; Fable 5.1 0,025× ; OpenAI 0,1-0,5×).
- **`glot translate --batch`** : 1er passage via l'API Message Batches d'Anthropic (-50 %), polling avec progression ;
  retries/escalade en appels directs. `custom_id` = hash du prompt ; l'id du lot (puis ses résultats) est gardé dans
  `.glot/batch.json` : relancer la même commande reprend le lot au lieu de repayer. Requêtes errored/expired => appel direct.
- **Estimation** (`translate::estimate_full`) : prompts réels + sortie attendue (formes plurielles de la locale cible,
  raisonnement ×3,5 min 400) + images `--visual` + juge + **appels de retry/escalade** (chacun renvoie tout le system
  prompt, escalade au prix de `fallback_model`) pondérés par leur probabilité ; max = tous les retries + escalade sur
  30 % des items, variance tokenizer 1,3×, aucun hit de cache. Cause de l'écart observé (0,071 $ réel > 0,060 $ max) :
  l'ancien calcul ne comptait qu'une marge de 10 % pour les retries, alors que chaque retry renvoie le system prompt et
  que l'escalade Sonnet coûte 20× Haiku.

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
           llm: {provider,model,fallback_model,judge_model,has_api_key,budget_usd,
                 anthropic_workspace_id,extra_headers},
           stats: {keys, locales: {<loc>: {translated,reviewed,stale,issues}}}}
LlmSettings = {provider,model,fallback_model?,judge_model?,api_key?(write-only),base_url?,budget_usd?,
               anthropic_workspace_id?,extra_headers?: {<name>: <value>}}
  anthropic_workspace_id / extra_headers : absents => conservés ; "" vide le workspace. Envoyés à chaque appel LLM
  du projet (jobs + Review API) ; `anthropic-workspace-id` = ce réglage, sinon env ANTHROPIC_WORKSPACE_ID.
  extra_headers = non secrets uniquement (≤ 20 ; 400 sur authorization/cookie/x-api-key/*key*/*token*/*secret*/*auth*).

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

## Insights (`crates/server/src/insights.rs`, migration `0024_insights.sql`, `packages/insights`)
« ROI incertain par langue » : demande réelle par langue, conversion par locale, expériences de langue.
Vie privée : aucun cookie, aucune IP / UA / route stockés ; uniquement des compteurs journaliers
`insight_daily(day, preferred, served, country, arm, event, n)` (`event="$view"` = une visite par session navigateur).
```
POST   /api/insights/{public_key}   (public, CORS *, corps ≤ 4 Ko, text/plain accepté)
         {langs:[navigator.languages], served, event?, country?, arm?, route?(ignorée)} -> {locale|null, arm|null}
         preferred = 1re langue valide de langs sinon Accept-Language (sous-tag primaire) ; country = body ou cf-ipcountry /
         x-vercel-ip-country / cloudfront-viewer-country. Si une expérience tourne pour cette langue, une visite tire un bras
         (traffic_pct) ; locale renvoyée = langue à servir (bras test). Les événements réutilisent le bras envoyé.
GET    /api/projects/{slug}/insights?days=30&goal=signup -> Insights (crée la clé publique si absente)
Insights = {public_key, days, goal, source_locale, offered[], keys, events[], total_visits,
  demand:[{language, visits, share, offered, conversions, conversion_rate, est_extra_conversions|null (par 30 j),
           est_cost_usd|null (dry-run translate::estimate, toutes les clés), top_countries:[[cc,n]]}],   // manquantes d'abord
  funnel:[{locale, is_source, visits, conversions, rate, ci_low, ci_high}],   // Wilson 95 %
  daily:[{day, visits, unserved}], experiments:[Experiment]}
POST   /api/projects/{slug}/insights/rotate-key -> {public_key}                      (admin ; l'ancienne clé → 404)
POST   /api/projects/{slug}/insights/experiments {locale, traffic_pct 1-100, days 1-90, goal?} -> Experiment (admin ;
         ajoute la locale aux target_locales ; 409 si déjà une expérience en cours pour la langue)
PATCH  /api/projects/{slug}/insights/experiments/{id} {status:"kept"|"removed"} -> 204 (removed retire la locale des cibles)
Experiment = {id, locale, traffic_pct, goal, started_at, ends_at, status, decided_at, test:Arm, control:Arm,
  diff, ci_low, ci_high (IC 95 % différence de proportions), required_per_arm|null (lift relatif +20 %, α .05, puissance .8),
  verdict:"collecting"|"keep"|"remove"|"inconclusive"}   // pas de verdict avant n requis ou date de fin
```
Client : `@glot/insights` (`init({key, endpoint, locale, onLocale})`, `track(event)`, `<script data-key>` auto-init,
`@glot/insights/server` `reportVisit(req, …)` pour un middleware Next/Edge). Dashboard : `/app/{slug}/insights`.

## Layout budgets (`glot measure`)
`glot measure --base-url … --routes …` renders the source locale with visual-qa, maps each element to its key
(rendered text → catalog regex, no instrumentation) and records the space it may use in `.glot/layout.json`
(versionable): `{ "<key>": { "max_width_px", "max_lines", "font": { family, size_px, weight, letter_spacing, transform } } }`.
Webfonts are converted to TTF in `.glot/fonts/`. `glot_core::measure` shapes text with rustybuzz (system-font
fallbacks per script) so `validate` can raise `too_wide` **without a browser**; the translation prompt carries the
budget ("must fit in 110px at Inter 14px/500 ≈ 16 Latin chars") and the retry loop fixes it before writing.
When a budget exists for a key it replaces the noisy length-ratio warning.
