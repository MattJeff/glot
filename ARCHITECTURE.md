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
| `packages/db` | `@glot/db` : lecture du contenu de base traduit (Prisma `withLocale`, Drizzle), migration `glot_translations` ; écrit par `glot db` (`crates/cli/src/db.rs`) |
| `packages/agent-qa` | Parité de tâche (`glot parity`, `crates/cli/src/parity.rs`) : agent Claude vision ou mock scripté, `glot.tasks.yaml`, rapport local `.glot/parity/` (pas d'upload) |
| `action/` | GitHub Action (`glot check` + `glot translate` + commentaire de PR) |

Helpers Node (`crates/cli/src/fit.rs::node_tool`) : `$GLOT_VISUAL_QA` / `$GLOT_AGENT_QA` (cli.js ou exécutable), sinon
(build debug seulement) `packages/*/src/cli.ts` du repo, sinon `glot-visual-qa` / `glot-agent-qa` dans le PATH, sinon
`npx -y --package=@glot-i18n/<nom>@<major.minor du binaire>` (scope changeable au build : `GLOT_NPM_SCOPE=@glot`).
Les packages sont compilés en `dist/` (esbuild, Node ≥ 20) et publiés par `scripts/publish-packages.sh` (`SCOPE`, `DRY_RUN=1`).

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

- **Structured outputs** : les lots de traduction (payload `"task": "translate"`) envoient le schéma strict
  `{"translations": {<clé>: string}}` (clés du lot en `required`, `additionalProperties: false`) via `output_config.format`
  (Anthropic, Message Batches compris) ou `response_format: json_schema` strict (OpenAI / compatible). 400 qui cite le
  schéma => renvoi en mode JSON simple ; le parse tolérant reste le repli. > 200 clés : pas de schéma.
- **Paliers de prix par requête** (`pricing::LONG_PROMPT`) : Claude Haiku 5.5 au-delà de 100K tokens de prompt
  (non caché + écritures + lectures de cache) passe de 0,10/0,50 $ à 0,50/2,50 $ ; Gemini 3.1 Pro au-delà de 200K.
  `translate::split` découpe les lots pour rester sous le seuil (heuristique ÷ 1,3) ; même découpe pour l'estimation,
  le pipeline et les custom ids `--batch`. Gemini (3 Flash, 3.1 Pro, 2.5 Flash) : provider `openai-compatible`,
  `base_url = "https://generativelanguage.googleapis.com/v1beta/openai/"`, `api_key_env = "GEMINI_API_KEY"`.

## Provenance (AI Act art. 50)
`LockEntry` porte `translated_at` / `reviewed_at` / `reviewed_by` (optionnels, RFC 3339 UTC), posés par
`Lockfile::record` (machine => `translated_at`, human => `reviewed_at` + `GLOT_REVIEWER`), conservés tant que la cible ne
change pas. `glot provenance export --format json|csv|html` (sans les textes), `glot provenance catalog` =>
`{locale}.provenance.json` lu par `@glot/runtime/provenance` (libellé « Translated by AI · reviewed on … »). Informer,
pas conclure : la doc ne tranche aucune question juridique.

## Fichiers projet (côté utilisateur)
`glot.toml` :
```toml
source_locale = "en"
target_locales = ["fr", "es", "de", "ja", "ar"]
files = ["messages/{locale}.json"]   # {locale} remplacé
format = "auto"                      # auto | json-nested | json-flat | xcstrings | strings | android | arb | po | yaml (auto = by extension, see glot_core::formats)
syntax = "icu"                       # icu | i18next | mf2 | printf (%@ %1$s %(name)s) | rails (%{name})

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

Qualité & coût (`glot_llm::{routing,tm}`, `crates/cli/src/queue.rs`, doc `web/content/docs/quality.md`) :
- Score écrit dans `glot.lock` `score` = juge (ou 100) − 15 par warning restant. `[qa] review_threshold` + `[qa.risk] <tag> = [globs de clés]`
  + note `risk: <tag>` → entrée écrite mais `needs_review: "<raison>"` (champ additif du lock) ; `glot queue` (list/approve/edit/
  export csv|xliff/import) l'efface ; `check` la signale en warning (toutes sorties, `github` inclus).
- TM `.glot/tm.json` (traductions validées non routées, `set`, approbations ; contexte = note) : exact → écrit sans LLM
  (`origin machine model "tm"`, pas sur `--force`) ; fuzzy ≥ 85 % (Levenshtein normalisé) → exemple dans `description`.
  `glot tm export|import` (TMX 1.4). `TranslateReport.{tm_reused,tm_saved,needs_review}`.
- `provider = "local"` (OpenAI-compatible, défaut `http://localhost:11434/v1`) : complétions taguées `local:<model>` → 0 $.
  `fallback_provider` (+ `fallback_base_url`, `fallback_api_key_env`) : escalade cross-provider (validateur après retries, ou
  score juge < `score_threshold`, valeur gardée seulement si elle valide).

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
       ?ondemand=1 : locale non livrée → 202 + job (voir « On-demand locales »), puis 200
GET|PUT /api/projects/{slug}/ondemand  {locales:["de"]|["*"], monthly_budget_usd?, per_day}
GET    /api/projects/{slug}/onboarding  -> {server_url,slug,source_locale,keys,first_push_at,locales:[{locale,translated,reviewed,missing,errors}],
         path,hidden,steps:[{id,done}],achievements:[{id,unlocked_at}]}   (onboarding.rs ; étapes dérivées de l'état réel,
         jalons first_push|first_locale_100|zero_errors|five_languages|glossary_set persistés dans `achievements`, jamais retirés)
PATCH  /api/projects/{slug}/onboarding  {path?: "agent"|"terminal"|"github", hidden?}
POST   /api/projects/{slug}/onboarding/token -> {token}   (token API perso nommé "<slug> (onboarding)", montré une fois)
GET    /badge/{slug}.json              (public si public_cdn ; endpoint shields.io ; 1er fetch => étape « badge README » cochée)
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

## Copy experiments (`crates/server/src/variants.rs`, migration `0057_variants.sql`)
« La traduction qui convertit le mieux », par (clé, locale). Bras 0 = traduction officielle ; 1-3 = variantes LLM
(même sens, tons direct/warm/urgent ; provider `mock` → `"<officielle> (<ton>)"`) **insérées seulement si validées**
(`keys::check` : placeholders/ICU/glossaire, + `measure::check` si `fit` fourni). Compteurs agrégés `variant_arms(views, conversions)`.
```
POST /api/projects/{slug}/experiments {key, locale, variants 2-3, explore_pct 1-100, min_n ≥ 50, goal?, fit?: measure::Fit}
       -> 201 VariantExperiment + rejected[] (admin ; 422 no_valid_variant ; 409 si déjà live pour clé+locale)
GET  /api/projects/{slug}/experiments -> VariantExperiment[] {…, status running|decided|promoted|stopped, winner,
       arms:[{arm, value, tone, views, conversions, alpha, beta, p_best, p_beats_control, lift}]}
POST …/experiments/{id}/promote -> écrit le gagnant via keys::write_translation (reviewed, historique) ; 409 sans gagnant
POST …/experiments/{id}/stop    -> 204
```
Service : piggyback sur `POST /api/insights/{pk}` (même rate limit). Visite → pour chaque expérience live de la locale
servie, `explore_pct` % des visiteurs tirent un bras (burn-in uniforme jusqu'à `min_n`/bras, puis top-two Thompson) ;
réponse `variants:[{id,key,arm,value}]` stockée en sessionStorage. Événement = goal → `variants:{id:arm}` crédite le bras.
Arrêt : un « look » chaque fois que le trafic total atteint `bras·min_n·2^k` ; gagnant si P(best) > 1 − 5 %/(bras·2^(k+1))
(budget d'erreur 5 % réparti sur bras et looks : simulation A/A ≈ 2 % de faux positifs, bon bras trouvé 100/100).
`decided` → seul le gagnant est servi, en attente de « promouvoir ». Source modifiée → plus servi.
Client : `variant(key)` (`@glot/insights`), `useGlotVariant(key)` / `<GlotVariants>` (`@glot/runtime/next-intl`, surcharge
les messages next-intl). Dashboard : `/app/{slug}/experiments`.

## Layout budgets (`glot measure`)
`glot measure --base-url … --routes …` renders the source locale with visual-qa, maps each element to its key
(rendered text → catalog regex, no instrumentation) and records the space it may use in `.glot/layout.json`
(versionable): `{ "<key>": { "max_width_px", "max_lines", "font": { family, size_px, weight, letter_spacing, transform } } }`.
Webfonts are converted to TTF in `.glot/fonts/`. `glot_core::measure` shapes text with rustybuzz (system-font
fallbacks per script) so `validate` can raise `too_wide` **without a browser**; the translation prompt carries the
budget ("must fit in 110px at Inter 14px/500 ≈ 16 Latin chars") and the retry loop fixes it before writing.
When a budget exists for a key it replaces the noisy length-ratio warning.
- **Exact keys (dev mode)** : text matching is ambiguous when two keys share a text (two « Save » → the first key gets
  both boxes; `glot measure` flags it as `ambiguous`). One line in `i18n/request.ts` :
  `messages: getGlotMessages(messages)` (`@glot/runtime/dev`), then run the app with `GLOT_DEV_KEYS=1`. Each message is
  prefixed with an invisible marker (U+2063, key UTF-8 bytes as variation selectors U+FE00–FE0F/U+E0100–E01EF, U+2063);
  visual-qa strips the markers before measuring (page renders like production) and sets `data-glot-key` on their element.
  Marker > text match. Off by default: messages are returned untouched. `createGlotDevTranslator(t, ns)` for a bare `t`.
- **Per occurrence** : each rendering (route × viewport × element) is kept in `occurrences: [{route, viewport, selector,
  max_width_px, max_lines, by: "marker"|"text"}]`; the key's budget is the tightest one (width × lines).
- Shaping passes `kern`, `liga`, `clig`, `calt` explicitly. Line fitting: CJK breaks between chars without a space,
  minimal kinsoku (no line starts with `、。，．」』）】〉》〕！？・…`, none ends with `「『（【〈《〔`; small kana may start
  a line, like Chromium's `line-break: normal`). Fixture `crates/core/tests/fixtures/browser-lines-ja.json` (macOS):
  never fewer lines than Chromium, ≤ 5 % off by one (`text-spacing-trim` not modelled).

## Framework setup (`glot i18nize`)
`glot i18nize` wires the runtime it detects, idempotently (a second run changes nothing), and every edit shows up in `--dry-run`:
- **Next App Router** → next-intl (`i18n/request.ts`, `proxy.ts`/`middleware.ts`, plugin, provider; cookie routing by default, `--routing prefix`). With `cacheComponents: true` (create-next-app ≥ 16.4 default) headers can't be read outside `<Suspense>`: `app/*` moves to `app/[locale]/*` (except `api/`, favicon/robots/sitemap/manifest, global-error; `../` imports and `LayoutProps<"/…">` fixed), `generateStaticParams` per locale, `request.ts` reads `next/root-params`, and the proxy rewrites unprefixed URLs to `/{cookie|default}/…`: same URLs, static pages per locale (`cargo test -p glot-cli --test first_contact`).
- **Next Pages Router** → built-in `i18n` + provider wrapping the whole `_app` tree.
- **React Router 7/8 (Remix)** → remix-i18next middleware: one i18next instance per request (`?lng` → cookie `glot-locale` → `Accept-Language`), `entry.server.tsx` with `I18nextProvider`, dynamic `<html lang>`. SSR renders the requested locale.
- **Astro (React islands)** → native i18n routing with `fallbackType: "rewrite"` (`/fr/...` reuses pages), `src/middleware.ts` sets the locale before islands render. Shared instance: safe for static builds, not for concurrent on-demand rendering.
- **Vite + React** → react-i18next init imported in the entry (SPA, client-side only).
Verified end to end on real apps in `examples/frameworks/` (`GLOT_FW_APPS=… cargo test -p glot-cli --test frameworks`).

## Insights rate limit
`POST /api/insights/{public_key}` is limited per project and per minute **across replicas**: one atomic
`INSERT … ON CONFLICT DO UPDATE … RETURNING hits` on `insight_rate` (one row per project, reset when the minute
changes, Postgres clock), merged into the key lookup so an accepted hit costs no extra round trip; keys over budget
are cached locally until the window ends, so floods never reach the database. Budget: `projects.insights_rate_per_min`,
else `GLOT_INSIGHTS_RATE_PER_MIN`, else 1200. Over budget → 429 `rate_limited` with `Retry-After`.

## Database content (`glot db`, `crates/cli/src/db.rs`, `packages/db`)
Contenu en base (catalogue, articles d'aide, plans, notifications) traduit dans une table shadow Postgres
`glot_translations("table", row_id, "column", locale, value, source_hash, status, model, updated_at)`, PK (table, row_id, column, locale),
`status` = `translated` | `stale` | `failed`. Config : `[[db.tables]]` de `glot.toml` (`table`, `primary_key`, `context`,
`columns = { col = "note" | { context, max_length } }`, `context_columns`).
- `glot db install` : table + fonction `glot_db_changed()` + trigger `glot_db` par table (args : pk puis colonnes). UPDATE d'une colonne
  localisée dont le hash change → `stale` ; DELETE → lignes shadow supprimées ; INSERT/changement → `NOTIFY glot_db`.
  Texte identique à `packages/db/migrations/0001_glot_translations.sql` (test).
- `glot db sync` : par table × locale, sélection = cellules non vides sans entrée, hash différent ou `stale` (`failed` au même hash ignoré) ;
  `Item` = source + `KeyContext` (description = table/colonne/notes, `max_length`, `neighbors` = reste de la ligne, `previous` = ancienne
  traduction) → `Translator` (validateur + glossaire + retries). Écriture par chunk (batch_size × 4) en transaction (reprise après
  interruption), budget vérifié avant chaque chunk via `translate::estimate`. `--watch` : `LISTEN glot_db`, debounce 300 ms.
- Hash = `glot_core::hash` = `left(encode(sha256(convert_to(v,'UTF8')),'hex'),16)` en SQL = `sourceHash` en JS.
- `@glot/db` : `createGlotDb(query).localize(table, rows, locale)` ne sert une traduction que si `status='translated'` **et** son
  `source_hash` = hash de la source courante, sinon valeur source. `glotPrisma` (extension `$extends`, `withLocale(l)` sur les lectures
  find*) et `glotDrizzle(db)` (`$client` node-postgres ou postgres.js).

## On-demand locales (`crates/server/src/ondemand.rs`, migration `0033_ondemand.sql`, `@glot/runtime/ondemand`, `packages/edge`)
Une locale non livrée devient une vraie page traduite côté serveur sans action du dev.
- `GET /cdn/{slug}/{locale}.json?ondemand=1` hors `target_locales` : job `done` pour la locale → 200 (catalogue, ETag, même cache) ;
  job en cours → 202 `{status:"translating"}`, `Retry-After: 5`, `no-store` ; sinon, si autorisé, crée un job `translate` normal
  (glossaire, validation, `llm.budget_usd`) avec `params.ondemand_locale` → 202. Sans `?ondemand=1` : 404 tant que rien n'est prêt.
- Garde-fous (verrou `FOR UPDATE` sur la ligne projet = correct entre réplicas) : liste blanche `projects.ondemand_locales`
  (`{*}` = toutes, `{}` = off, défaut) → 403 `ondemand_not_allowed` ; `ondemand_per_day` jobs / 24 h glissantes (défaut 5) → 429 +
  `Retry-After` ; `ondemand_budget_usd` = somme `cost_usd` des jobs on-demand du mois → 402 `ondemand_budget_exhausted`.
  Pas de table : l'état est dans `jobs` (index partiel `jobs_ondemand`). Job `failed` → redemandable (compte dans le quota).
- `createOnDemandLoader({server, project, source?})` → `load(locale)` : catalogue (fusionné sur la source) ou `null` (202/erreur/glot
  injoignable → rendre la source ; le fallback on-device du navigateur prend le relais). Cache mémoire, requêtes dédupliquées,
  stale-while-revalidate avec `If-None-Match` ; un `null` est redemandé après `Retry-After`.
- `packages/edge` : `route()` (préfixe `/{locale}/`, sinon cookie `glot_locale` / Accept-Language → 302 pour les navigations HTML),
  Worker Cloudflare (pages statiques : remplit les éléments `data-glot="clé"` via HTMLRewriter) et `glotProxy` Next 16 (`proxy.ts`,
  rewrite + en-tête `x-glot-locale`).

## Branch coverage (`glot coverage`, `glot_core::branches`)
`branches::enumerate(msg, locale, syntax)` (child module of `validate`, walks its AST) : produit cartésien des cas de chaque
variable de sélection — `plural` : catégories CLDR de la locale avec un témoin (`witness`, ex. pl few=3 many=11 other=1.5,
ar 0/1/2/3/11/100) + chaque `=N` ; `selectordinal` : `ordinal_witnesses` (catégories ordinales CLDR de la locale, ICU4X) + clés du
message ; `select` : chaque clé + `other`. Variables non atteintes ignorées (dédup), plafond `MAX_BRANCHES`. Texte rendu :
`=N` > catégorie > `other`, `#` = témoin, autres placeholders = `render_sample`, balises retirées. MF2 : `.match` à un
sélecteur exact ; multi-sélecteurs → texte du 1er sélecteur. i18next non couvert (pluriels = clés `_few`).
CLI (`crates/cli/src/coverage.rs`) : mesure chaque branche (`measure::check`) ; `--render` écrit `.glot/coverage/harness.json`
`{locale: html}` (une boîte par branche avec le budget de `.glot/layout.json`, `data-glot-key="clé#branche"`) et lance
`glot-visual-qa --harness` (le fragment remplace le body de la page `--base-url` + 1re route : CSS/polices de l'app).
`.glot/coverage.json` : `{locale: {key: {branch_id: {text_hash, issue|null}}}}` ; une branche n'est « vue » que si son texte
a le même hash. Rapport par locale × catégorie (branches, vues, en échec, trop larges) ; exit 1 si échec.
Pluriels : `glot_core::plural` = ICU4X (`icu_plurals`, données CLDR compilées) pour `categories` (cardinaux),
`ordinal_categories` et `category(locale, n, ordinal)`, toutes locales CLDR ; langue inconnue de CLDR → `[one, other]`.
`validate` exige les catégories ordinales de la locale pour `selectordinal` ; un sélecteur traduit (`un`, `autre`) donne
« `un` is a translated keyword, use `one` » au lieu d'une catégorie manquante.

## Register variants (`[variants]`, `glot_core::variants`)
`glot.toml` : `[variants] fr = ["formal", "informal"]` → locales `fr-x-formal`, `fr-x-informal` (extension privée BCP 47),
catalogues via `files` comme les autres. Lu à part (`variants::load`) pour ne pas casser les littéraux `Config`.
`glot translate` (puis `crates/cli/src/variants.rs`) : après la base, delta via `lock.plan(base, [variante])`, requête
`source_locale = base`, `style` = style + `variants::instruction` (registre), glossaire de la base recopié ; lock enregistré
contre le texte de la base. `glot check` ajoute `variants::check` (validate base → variante, `untranslated` ignoré).
Runtime : une locale comme une autre (cookie / préférence utilisateur, ex. `NEXT_LOCALE=fr-x-informal`).

## SEO multilingue (`glot seo`, `crates/cli/src/seo.rs` + `seo/`, `glot_llm::seo`)
- `glot seo audit --base-url … [--routes a,b | --sitemap] [--url-pattern "/{locale}{route}"] [--json]` : HTML serveur
  (ce que voit un crawler, sans navigateur) de chaque (route, locale) + cibles hreflang. Erreurs : HTTP ≠ 200, `<html lang>`,
  canonical absent / vers une autre locale, hreflang absent / incomplet / invalide (`jp`, `en-UK`) / sans auto-référence /
  non réciproque / cassé / mauvaise langue, title absent ou identique à la source, page indexable ≥ 90 % identique à la
  source (« scaled content »). Warnings : x-default, title > 60 / description > 160 (largeur SERP, CJK ×2), sitemap absent ou
  URL manquante, robots.txt sans sitemap, slug. Findings rattachés à la clé du catalogue (texte == valeur). Exit 1 si erreur.
- `glot seo fix [--dry-run] [--yes]` (Next App Router, après i18nize, via `setup::SetupPlan`, idempotent) : `x-glot-pathname`
  dans `i18n/proxy.ts`, `i18n/seo.ts` (`seoAlternates()` : canonical + hreflang + x-default de la requête, `SITE_URL`),
  `alternates` inséré dans le `generateMetadata` du layout racine (ou `metadata` converti), `app/sitemap.ts` (routes
  statiques × locales avec alternates), `app/robots.ts`.
- `glot seo keywords [--locale] [--keys] [--apply]` : clés notées « … in generateMetadata » ; le LLM (tâche
  `seo_keywords`) donne les requêtes que tapent les natifs + 2-3 variantes ≤ budget SERP ; termes stockés dans
  `lock.notes` (`SEO fr: …`) ; `--apply` écrit la 1re variante qui passe `set_translation` (validation + glossaire).

## Feedback (`crates/server/src/feedback.rs`, migration `0040_feedback.sql`, `packages/feedback`)
Native speakers flag a translation in production → correction proposed → human accepts → optional GitHub PR.
Prior art / FTO notes: `docs/prior-art.md` (one key per correction, no propagation to other occurrences).
```
POST   /api/feedback/{public_key}   (public, CORS *, ≤ 8 Ko, text/plain accepté ; même clé publique qu'Insights)
         {locale, text, kind?:"wrong"|"awkward"|"unclear", suggestion?, url?, dom?, key?} -> 202 {matched}
         Clé = `key` si connue, sinon index inverse du catalogue de la locale (ICU {…} = joker, balises ignorées ;
         sinon plus courte valeur contenant le texte, ≥ 4 car.). Non trouvé => {matched:false}, rien stocké.
         Vie privée : url sans query/fragment, dom = chemin de balises (pas de texte), ni IP/UA/cookie.
         Rate limit partagé Postgres (`feedback_rate`, même schéma qu'insights) : GLOT_FEEDBACK_RATE_PER_MIN (30) → 429.
         Regroupement par (key, locale) ; au seuil (`threshold`, défaut 3) => proposition en tâche de fond.
GET    /api/projects/{slug}/feedback -> {settings: FeedbackSettings, groups: [Group]}
PUT    /api/projects/{slug}/feedback/settings {threshold?, github_repo?, github_base?, catalog_path?, lock_path?,
         github_token? (write-only chiffré comme la clé LLM ; "" efface)} -> FeedbackSettings   (admin)
POST   /api/projects/{slug}/feedback/{id}/propose  -> Group   (pipeline de traduction gardé avec `previous` = cible
         actuelle et signalements dans la description, puis rétro-traduction Review API)
POST   /api/projects/{slug}/feedback/{id}/accept {value?} -> Group  (422 si le validateur trouve une erreur ;
         écrit origin=human, status=reviewed)
POST   /api/projects/{slug}/feedback/{id}/reject -> Group
POST   /api/projects/{slug}/feedback/{id}/pr -> Group (pr_url) (admin, groupe accepté) : Git Data API —
         1 commit (catalogue `catalog_path` avec {locale} + `lock_path`) sur `glot/feedback-{locale}-{id8}`, PR avec
         tableau avant/après. Base API : GLOT_GITHUB_API (défaut https://api.github.com).
FeedbackSettings = {public_key, threshold, github_repo|null, github_base, catalog_path, lock_path, has_github_token}
Group = {id, key, locale, source, current, status:"open"|"proposed"|"accepted"|"rejected", count, proposal, previous,
         back_translation, verdict, explanation, issues[], pr_url, updated_at, reports:[{kind,text,suggestion,url,dom,created_at}]}
```
Widget `@glot/feedback` (< 4 Ko min., Shadow DOM, `<dialog>` natif) : sélection de texte → « Improve translation »,
ou événement `glot:improve` du badge runtime (detail {key?, text?}), ou `open(text, key)`. `<script data-key data-endpoint>`
auto-init ; `labels` à passer traduits. Dashboard : `/app/{slug}/feedback`.

## Assets : emails, SVG, OG (`glot assets`, `crates/cli/src/assets.rs`, `crates/extract/src/{email,svg}.rs`)
`glot assets [i18nize|translate|preview|check] [--emails] [--svg] [--og]` (défaut : les 4 étapes, idempotent).
- **i18nize** : JSX (`emails/*.tsx`, `og/*.tsx`) via `glot_llm::i18nize` (clés = namespace du chemin, `emails.<template>.*`,
  `og.<name>.*`) + `export const subject = "…"` → `(t) => t("emails.<t>.subject")`. HTML/MJML/SVG jamais réécrits : scanner
  de balises (`email::scan_markup`) → clés stables (`id` de l'élément sinon slug du texte), le fichier est la source de vérité
  de son préfixe (`emails.<t>.` / `svg.<name>.` : clés disparues supprimées). `{{var}}` ⇄ `{var}`.
- **Budgets** dans `.glot/layout.json` (même format que `glot measure`) : `<text>` SVG (`data-max-width`, sinon place
  restante dans le viewBox selon `text-anchor`, 1 ligne), blocs OG (largeur × `lineClamp`), sujet (60 chars Latin moyens à
  Helvetica 14px) et preheader (100). `glot translate`/`check` les appliquent : `too_wide` → retry avant écriture.
- **translate** : `Workspace::translate` restreint aux clés des assets (lock, glossaire, validateur inchangés).
- **preview** : évaluation statique du JSX (`email::render_jsx` : composants React Email, `t()`/`t.rich()`, ICU minimal,
  `PreviewProps`) → `.glot/emails/<t>/<locale>.html` ; HTML/MJML substitués span par span (`lang`/`dir` posés) ;
  SVG → `public/img/<locale>/<name>.{svg,png}` ; OG → mise en page colonne flex (`svg::og_svg`, retour à la ligne
  avec `glot_core::measure`) → `public/og/<locale>/<name>.png`. Rendu PNG : resvg (polices système, `.glot/fonts/`).
- **check** : `Workspace::check` sur les clés des assets + liens identiques à la source, sujet ≤ 60 / preheader ≤ 100,
  `<Html dir>` absent pour une locale RTL (warning), largeur réellement rendue par resvg (bbox usvg) > budget, OG plus
  haut que l'image. Exit 1 si erreur.

## Emails transactionnels (`crates/server/src/email.rs`, migration `0070_email.sql`, catalogue `crates/server/emails/`)
API HTTP Resend (reqwest), off si `RESEND_API_KEY` absent (`AppState.mailer = None`). Outbox Postgres `email_outbox` : rendu
(HTML + texte, `{locale}.json` traduit par glot, repli en) à l'enqueue, `idem_key` UNIQUE (doublons ignorés), budget
`GLOT_EMAIL_RATE_PER_HOUR` par destinataire ; worker `SKIP LOCKED`, `Idempotency-Key` = id de la ligne, 429/409/5xx/réseau →
retry 30 s × 2^n (6 essais), autre 4xx → `failed` ; corps effacés après envoi/échec (liens à usage unique).
```
POST /api/auth/verify-email {token} -> 204        (48 h, usage unique ; change users.email si changement d'adresse, notifie l'ancienne)
POST /api/auth/verify-email/resend -> 204          (session ; 503 email_disabled)
POST /api/auth/forgot-password {email} -> 204      (toujours 204 ; 503 si email off)
POST /api/auth/reset-password {token,password}     (1 h, usage unique, adresse inchangée ; supprime toutes les sessions)
PUT  /api/me/password {current_password,password}  (garde la session courante) · PUT /api/me/email {email,password} -> {pending}
GET  /api/me -> User + {email_verified, email_enabled, locale, notify_job_done} · PATCH /api/me {locale?, notify_job_done?}
```
Tokens : sha256 dans `email_tokens`. `jobs.created_by` (POST …/jobs) reçoit `job_done`/`job_failed` si `notify_job_done`.
Web : `/verify-email`, `/forgot-password`, `/reset-password`, bannière (`VerifyEmailBanner`), `/app/settings/account`.

## MCP distant (`crates/server/src/mcp_remote.rs` + `mcp_remote/oauth.rs`, migration `0060_mcp_remote.sql`, doc `web/content/docs/mcp-remote.md`)
- `POST /mcp` : rmcp Streamable HTTP **stateless** (`NeverSessionManager`, réponses JSON), 1 projet par connexion. Outils =
  appels directs des handlers REST (`keys::*`, `jobs::*`, `review::*`, `sync::pull`…) → mêmes rôles/validation/historique.
  `translate` = job serveur + attente ≤ 50 s ; `translate_repo` = PR GitHub (catalogues source+cibles + `glot.lock`) via
  `feedback_settings` (repo/token chiffré) ; pas d'i18nize/extraction côté serveur.
- Auth (`guard`) : `Bearer glot_…` (API token, `?project=slug` ou `X-Glot-Project`, tous scopes) ou `Bearer glot_at_…` (OAuth,
  projet + scopes fixés au consentement, audience RFC 8707). 401 → `WWW-Authenticate: Bearer resource_metadata=…` ;
  `tools/call` hors scope → 403 `insufficient_scope` (le guard lit le corps JSON-RPC) ; `tools/list` filtré par scopes.
  Scopes : `read` / `translate` (`translate`, `review`) / `write` (`add_string`, `set_translation`, `glossary_upsert`, `translate_repo`).
- OAuth 2.1 : `/.well-known/oauth-protected-resource` (RFC 9728), `/.well-known/oauth-authorization-server` (RFC 8414), DCR
  `/oauth/register` (clients publics), `/oauth/authorize` → redirige vers `{GLOT_WEB_ORIGIN}/oauth/authorize` (consentement
  `web/app/[locale]/oauth/authorize`, API session `GET|POST /api/oauth/authorize`), code 5 min usage unique + PKCE S256, `iss`
  (RFC 9207), `/oauth/token` (access 1 h, refresh 30 j avec rotation), `/oauth/revoke` (RFC 7009, supprime le grant). Secrets
  stockés en sha256 (`oauth_codes`, `oauth_grants`).
- Sécurité : Origin validé par rmcp (`GLOT_WEB_ORIGIN` + `GLOT_MCP_ALLOWED_ORIGINS`), Host = `GLOT_PUBLIC_URL` si défini ;
  rate limit `mcp_rate` par token (`GLOT_MCP_RATE_PER_MIN`, défaut 120) ; audit `mcp_audit` (outil, ok, ms, client ; jamais les arguments).

## Billing (`crates/server/src/billing.rs`, migration `0080_billing.sql`, doc `docs/billing.md`)
Off unless `STRIPE_SECRET_KEY` is set (self-hosted = no limit; CLI / local MCP never reach it). One account per user, covering
the projects it owns. Monthly counters `billing_usage` + plan limits (`GLOT_PLAN_LIMITS` override) → 402 `quota_exceeded` /
`plan_required` / `plan_limit` with an upgrade link, enforced in `projects::create`, `cdn::serve`, `insights::ingest`,
`mcp_remote::guard`, `ondemand::gate`, `jobs::create` (glot's LLM key only; BYOK never counted; overage → Billing Meter
`glot_llm_strings` when the subscription is metered). Stripe: Checkout / Portal / invoices / `POST /api/billing/webhook`
(signature, idempotent `stripe_events`, out-of-order guard), API version `2026-09-30.endive`, prices by lookup key
(`deploy/stripe-setup.sh`). `past_due` keeps the plan until `grace_until` (first failed invoice + 7 days).
Web: `/pricing`, `/app/settings/billing`.

## Connect GitHub (`crates/server/src/github_app.rs`, migration `0101_github_app.sql`, doc `web/content/docs/github.md`)
Une GitHub App par déploiement, créée par le flux manifest (`/admin/github-app`, admins = `GLOT_ADMIN_EMAILS`) ; app_id,
clé privée, webhook secret, client secret chiffrés (GLOT_SECRET_KEY) dans `github_app`. Permissions : contents:write,
pull_requests:write, metadata:read ; webhooks push, pull_request (+ installation, implicite).
```
GET  /api/github/app                      -> {configured, can_admin, slug, app_id, html_url, webhook_url}
POST /api/github/app/manifest {name?,org?} -> {action, manifest}   (admin ; la page POSTe le formulaire vers github.com)
GET  /api/github/app/callback?code&state  (public : state usage unique 1 h → POST /app-manifests/{code}/conversions)
GET  /api/projects/{slug}/github          -> {app|null, installations:[{id,account}], link|null, job|null}
POST /api/projects/{slug}/github/connect  -> {install_url, authorize_url}   (admin du projet)
GET  /api/github/callback?code&state      (public : code OAuth → GET /user/installations = preuve d'accès, jamais l'installation_id de l'URL)
GET  /api/projects/{slug}/github/repos?installation_id -> [{full_name, default_branch, private}]
PUT  /api/projects/{slug}/github/link {installation_id, repo, branch?} -> {link, job}  (job github_localize)
POST /api/projects/{slug}/github/run -> Job ; DELETE …/link
POST /api/github/webhook                  (X-Hub-Signature-256 ; push sur la branche liée → job github_update ;
                                           pull_request closed → oublie la PR ; installation deleted → supprime les liens)
```
Jobs (`jobs` existants, SSE inchangé, `kind` = `github_localize` | `github_update`, `progress` = étapes /6 : token, clone,
localize, sync, push, pr ; pas de facturation `llm_strings` ni d'email) : token d'installation limité au repo (jamais en argv/log :
`http.extraHeader` via `GIT_CONFIG_*`), `git clone --depth 1 -c core.symlinks=false` dans `$TMPDIR/glot-gh-{job}` (supprimé au
Drop), taille max `GLOT_GITHUB_MAX_REPO_MB`, `glot.toml` refusé si `files`/`[extract].include` sortent du clone, `glot init` +
`glot localize --yes --no-install` (`GLOT_BIN`, env vidé sauf PATH/HOME/clé LLM du projet, `ulimit -v/-t`, groupe de processus tué
au timeout), `sync::apply_push` (règles de `glot push`), commit `{slug}[bot]` hors `.github/workflows`, push `--force` sur
`glot/localize` (ou la branche de la PR ouverte, sinon `glot/update`), une seule PR ouverte (PATCH + commentaire de statut).
Rate limit : `GLOT_GITHUB_JOBS_PER_HOUR` (20) par installation, un seul job en attente par projet (verrou advisory).
UI : `<ConnectGitHubCard slug />` (`web/components/github/`), page `/app/{slug}/github`, admin `/admin/github-app`.

## Live review (`crates/server/src/review_live/`, migration `0120_review.sql`, contrat `docs/design/live-review-contract.md`)
Annotations ancrées sur le site du client (widget, jeton `glr_`) ou depuis la page Review du SaaS (membre) : fils, réactions,
captures (≤ 1 Mio en base), filtres, assignation, historique `review_events` poussé en SSE (`pg_notify('glot_review')`, un
LISTEN par process, replay `Last-Event-ID`), « appliquer » = `keys::write_translation`, édition manuelle du texte (multi-langues
atomique, revert, suppression, création de clé), conversion d'un groupe feedback. Préfixes `/api/projects/{slug}/review/…`
(session/API token) et `/api/review/w/…` (liens `read|comment|edit` expirables/révocables, sessions membre 12 h). CORS widget
strict (`review_settings.origins` + `GLOT_WEB_ORIGIN`, puis origine liée au projet du jeton), rate limit partagé `review_rate`
(`GLOT_REVIEW_RATE_PER_MIN`, 120), quota billing `review_annotations`, markdown rendu côté serveur en HTML sûr (`body_html`).
