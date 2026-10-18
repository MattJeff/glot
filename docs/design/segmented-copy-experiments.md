# Segmented copy experiments — conception (v1, 2026-10-18)

> Étend les *Copy experiments* existants (`crates/server/src/variants.rs`, migration `0057_variants.sql`) :
> même cœur bayésien (Beta, top-two Thompson, `decide`/`next_look`), mêmes garde-fous (`keys::check`, glossaire,
> `measure::check`), même promotion (`keys::write_translation`). Ce qui change : **groupes de clés**, **segments**,
> **décision à l'edge/SSR sans flicker**, **revenus**, **holdout**.

## 0. Constat sur l'existant (ce qui bloque aujourd'hui)
- Tirage dans `POST /api/insights/{pk}` → **client-only** : 1er rendu officiel puis swap (`useEffect` + sessionStorage) = flicker sur un hero.
- Une expérience = (1 clé, 1 locale) : pas de « titre + sous-titre + CTA » cohérent, ni de même idée dans toutes les langues.
- Aucun segment (`insights::country` n'alimente qu'`insight_daily`), et glot est sur le chemin critique de chaque visite.

## 1. Modèle

**Contexte visiteur** (calculé à l'edge, jamais envoyé brut à glot) :

| Attribut | Source | Valeurs |
|---|---|---|
| `country`, `region` | `cf-ipcountry` / `x-vercel-ip-country(-region)` / `cloudfront-viewer-country` | ISO 3166 |
| `locale` | locale servie (`x-glot-locale`) | locales du projet |
| `device` | `sec-ch-ua-mobile`, sinon regex UA | `mobile` / `tablet` / `desktop` |
| `source` | `utm_source`/`utm_medium`, sinon hôte du referrer classé | `search` / `social` / `email` / `paid` / `direct` / `referral` |
| `visitor` | uniquement avec consentement (flag localStorage) | `new` / `returning` / `unknown` |
| `plan`, `logged_in` + `custom.*` | fournis par l'app (`glotContext({plan:"pro"})`) | **énumérés**, ≤ 20 valeurs déclarées, sinon ignorés (anti-PII) |

**Segment** = règle nommée, conjonction de prédicats : `{"all":[{"attr":"country","in":["DE","AT","CH"]},{"attr":"source","eq":"search"}]}`. Ordre de priorité, premier qui matche ; le reste tombe dans `*`.

**Expérience** = un **groupe de clés** (ex. `home.hero.{title,subtitle,cta}`) × **toutes les locales cibles** × une **liste de segments ciblés** (cellules). Un **bras** = une *intention* (« preuve sociale », « bénéfice direct »…) avec un texte par (clé, locale). Bras 0 = textes officiels. Une seule expérience live par clé (index unique conservé).

**Allocation** (`allocation`) :
- `fixed` : poids fixes (A/B classique), seule option qui permet le SRM strict.
- `bandit` : top-two Thompson existant, **une posterior globale**.
- `segmented` (« contextual bandit » honnête) : top-two Thompson **par cellule**, avec prior partiel tiré du global (Beta(α_g·k, β_g·k), k = 100 visiteurs équivalents). Garde-fous : une cellule ne s'écarte des poids globaux qu'après `min_n` par bras ; plancher 10 % par bras en exploration ; max 4 cellules (MVP) ; décision jamais avant 7 jours (saisonnalité hebdo). Un vrai bandit contextuel (LinUCB sur features) est **hors sujet** : le trafic d'une landing ne le nourrit pas.

**Objectifs** : événement `glot.track("signup")` (existant) ou revenu `track("purchase", {value: 49, currency: "EUR"})` → conversions + somme et somme des carrés du revenu par bras/cellule (approx. normale du revenu par visiteur). Une métrique décisionnelle, d'autres en garde-fou (ex. `refund` ne doit pas monter).

**Holdout global** : `projects.xp_holdout_pct` (défaut 0, conseillé 5 %). Ces sessions ne voient **aucune** variante ni personnalisation : mesure l'effet cumulé de tout ce qui a été promu. Stocké comme une pseudo-expérience `kind='holdout'`.

## 2. Livraison sans redeploy ni flicker

```
Navigateur ──► Edge (Next proxy / CF Worker) ──► App SSR (next-intl)
                 │ decide(config, ctx) local, 0 appel bloquant à glot
                 │ config = GET /cdn/{slug}/experiments.json (ETag, max-age 60, SWR 600, cache isolate)
                 └► waitUntil(POST /api/insights/{pk} {exposures})  (asynchrone)
```
1. **Config publiée** `GET /cdn/{slug}/experiments.json` (nouvelle route à côté de `cdn::serve`, mêmes règles public/privé, `billing::CDN`) : segments, expériences live, **poids courants par cellule**, textes des bras par locale (seulement les clés en test : quelques Ko). Le serveur recalcule les poids à chaque *look* (bandit « par lots ») au lieu de tirer à chaque hit.
2. **Décision edge** : `decide(config, ctx, rand)` pure (`packages/edge/src/experiments.ts`), partagée Next proxy / Worker. Résultat passé à l'app en en-tête de requête `x-glot-xp` (`{id:arm}`) + overlay `{key:value}` pour la locale.
3. **SSR next-intl** : dans `getRequestConfig`, `messages: await withExperiments(messages)` (lit l'en-tête, réutilise `withVariants` de `packages/runtime/src/variants.ts`). Le HTML part déjà avec la variante ; `NextIntlClientProvider` reçoit les messages fusionnés → hydratation identique, **zéro flicker**.
4. **Worker Cloudflare** (sites statiques) : le `HTMLRewriter` sur `[data-glot]` existe déjà ; il applique l'overlay.
5. **Cache** : une page en test varie par bras → `Cache-Control: private` sur les routes concernées (MVP). v2 : pages statiques via *precompute* (bras encodé dans un segment de chemin réécrit, combinaisons bornées).
6. **Fallback** : config absente/expirée (> 24 h) ou glot injoignable → bras 0, aucune exposition comptée. Timeout fetch config 150 ms, jamais bloquant si un cache existe.
7. **Client-only** (dernier recours, SPA) : chemin actuel `@glot/insights` inchangé.
8. **SEO** : Google qualifie explicitement de cloaking « servir toujours l'original à Googlebot » selon le user-agent. Donc **on ne décide pas selon l'UA** (contrairement à l'idée « bots → contrôle ») : même URL, même canonical, variantes sémantiquement équivalentes, durée limitée. L'UA bot sert seulement à **ne pas compter** l'exposition. Sans cookie, un crawler voit un tirage aléatoire comme tout visiteur.

## 3. Qualité
- Génération : le LLM écrit le bras **en langue source** à partir d'une intention + segment (ton, longueur, preuve sociale locale), puis chaque locale passe par le **pipeline de traduction normal** (glossaire, `validate`, retries, budget `.glot/layout.json` via `measure::check`). Un bras n'est activable que si **toutes** les (clé, locale) valident ; sinon il est listé dans `rejected` (comme aujourd'hui).
- Faits : la preuve sociale ne peut citer que des faits déclarés (`facts` de l'expérience : « 2 000 équipes », « note 4,8 »). Contrôle déterministe : tout nombre, %, devise ou date absent de la source et des faits → rejet.
- Légal : les clés taguées par `[qa.risk]` (`pricing`, `legal`) sont **interdites** en expérience ; prompt existant « never add claims, prices or urgency » conservé.
- Revue humaine : bras en `draft` jusqu'à approbation si `review: true` ou clé à risque ; réutilise la file `needs_review`.

## 4. Statistiques
- **Moteur** : `posterior`, `decide`, `next_look` inchangés, appliqués **par cellule**. Le budget d'erreur de 5 % est partagé entre bras × cellules × looks : seuil `1 − 5 %/(bras·cellules·2^(look+1))`. Test A/A simulé à étendre aux cellules.
- **Puissance** : à la création, trafic du segment estimé sur 30 j d'`insight_daily` (pays × locale) + `required_per_arm` (`insights.rs`) → « ≈ 19 jours pour détecter +20 % ». > 90 jours → la cellule est fusionnée dans `*` (avertissement bloquant, contournable).
- **Global vs segment** : on calcule P(best) global et par cellule. « Gagnant par segment » seulement si : la cellule a un gagnant sous son seuil **et** P(lift_cellule > lift_global) > 95 % pour ce bras. Sinon on recommande « promouvoir partout ».
- **Tranches exploratoires** (pays top 20, device, source) : affichées, jamais décisionnelles, marquées « exploratoire », correction Benjamini-Hochberg sur `2·min(p, 1−p)` de P(beats control).
- **SRM** : χ² des expositions vs poids attendus (mode `fixed`, phase de burn-in uniforme et split holdout) ; p < 0,001 → bandeau rouge, décisions gelées.
- Limite : poids variables dans le temps → biais si le taux de base dérive ; mitigé par looks espacés + 7 jours minimum.

## 5. Vie privée (glot informe, ne conclut pas)
- **Aucune IP, UA, route ni identifiant stockés** : uniquement des compteurs `(expérience, cellule, bras, jour)`. Les attributs app/custom ne quittent jamais l'edge : seule la cellule matchée remonte.
- **Persistance** (`persistence` du projet) :
  - `none` (défaut, = existant) : rien sur le terminal côté edge ; tirage à chaque document, collé ensuite par sessionStorage côté client. Limite : l'edge ne lit pas sessionStorage, donc une page rechargée peut changer de bras (contamination, effet dilué). Correct pour une landing, pas pour un tunnel multi-pages.
  - `session` (recommandé pour SSR) : cookie de session first-party `glot_xp` contenant **seulement** `id.bras` (pas d'identifiant, pas d'expiration), statistiques agrégées pour l'éditeur seul : vise les conditions d'exemption « mesure d'audience » de la CNIL, à valider par le DPO du client.
  - Attributs persistants (`visitor=returning`, source first-touch) : **seulement après consentement** (`glot.consent(true)`), localStorage.
- Opt-out : `glot.optOut()` / `navigator.globalPrivacyControl` / DNT → bras 0, rien compté.

## 6. UX dashboard (`/app/{slug}/experiments`)
**Création en 4 étapes** (dialog, réutilise les composants existants) :
1. *Quoi* : choisir des clés (recherche ou cliquer dans l'aperçu `/preview`) ; suggestion de groupe par préfixe (`home.hero.*`).
2. *Pour qui* : constructeur de segment (pays, source, device, plan…) avec **trafic estimé en direct** et durée estimée.
3. *Variantes* : intentions → génération IA, édition inline, **grille clé × locale** avec badges validateur / glossaire / largeur.
4. *Objectif* : événement ou revenu, « A/B fixe » vs « Optimiser automatiquement », holdout, durée → Lancer.

**Aperçu** : la matrice de locales (`/preview`) gagne un sélecteur segment × bras ; force via `?glot_xp=<id>.<bras>` (non compté).

**Résultats** lisibles : une phrase par cellule — « Variante B : +12 % d'inscriptions (probablement entre +4 % et +20 %), confiance 97 %. Encore ~3 jours. » ; feux tricolores ; graphique de posteriors existant replié en « détails ». Bandeaux SRM / puissance insuffisante.

**Actions** : « Promouvoir partout » (= `promote` existant, toutes locales via `write_translation`, historique, CDN/PR) vs « Promouvoir pour ce segment » → statut `personalized` : 100 % du bras gagnant dans la cellule, sans comptage hors holdout, rappel de re-test à 90 jours, désactivé automatiquement si la source change (règle existante).

## 7. DX
- **SDK** : pas de nouveau hook côté appel. `withExperiments(messages)` dans `i18n/request.ts` + `glotProxy({experiments: true})` = transparent pour tous les `t()`. `useGlotVariant` reste pour le client-only. `track(event, {value, currency})`. `glotContext({plan})` côté serveur pour les attributs app. `glot i18nize` ajoute ces lignes (idempotent).
- **MCP** (distant, scope `write`) : `experiment_create {keys|prefix, segment, goal, intents}`, `experiment_list`, `experiment_results`, `experiment_promote {scope: all|segment}`. « Crée une expérience sur le hero pour le trafic Google DE » → `prefix: "home.hero"`, `segment: {country:["DE"], source:"search"}`, retourne estimation de durée + bras rejetés.
- **CLI** : `glot labs experiments list|create|results|promote|stop` (REST, token `GLOT_TOKEN`).
- **Git** : `glot.experiments.toml` **oui, mais v2** et seulement pour les *définitions* (segments, clés, objectifs, faits), pas pour l'état (poids, compteurs). Les promotions « partout » reviennent déjà dans les catalogues ; les personnalisations seraient exportées dans `messages/{locale}.segments.json` lu par `withExperiments`.

## 8. Plan

### MVP (1 itération, agents parallèles par zone)
| Agent | Fichiers | Contenu |
|---|---|---|
| Server | `crates/server/migrations/0110_segmented_experiments.sql`, `variants.rs`, `cdn.rs`, `insights.rs` | Tables ci-dessous ; création multi-clés × locales ; `GET /cdn/{slug}/experiments.json` ; `Hit.exposures` + `Hit.value` ; évaluation par cellule ; SRM ; promote `scope` |
| Edge | `packages/edge/src/{experiments.ts,next-proxy.ts,worker.ts}` | `decide()` pur + tests ; en-tête `x-glot-xp` ; cookie `session` optionnel ; overlay HTMLRewriter |
| Runtime | `packages/runtime/src/{next-intl-server.ts,variants.ts}`, `packages/insights/src/index.ts` | `withExperiments`, `track(value)`, `consent/optOut`, reprise de l'assignation SSR dans sessionStorage |
| Web | `web/app/[locale]/app/[slug]/experiments/{page,parts}.tsx`, `web/lib/api.ts`, messages | 4 étapes, résultats par cellule, deux boutons de promotion |
| Stats/QA | tests `variants.rs` | A/A par cellules, SRM, contrôle des faits/nombres |

Migration `0110` :
```sql
CREATE TABLE xp_segments (id uuid PRIMARY KEY, project_id uuid REFERENCES projects ON DELETE CASCADE,
  name text NOT NULL, rule jsonb NOT NULL, UNIQUE (project_id, name));
ALTER TABLE variant_experiments ADD keys text[], ADD cells uuid[] NOT NULL DEFAULT '{}',
  ADD allocation text NOT NULL DEFAULT 'bandit', ADD facts jsonb, ADD kind text NOT NULL DEFAULT 'copy';
  -- locale = '*' pour les nouvelles ; 'personalized' rejoint les statuts
CREATE TABLE variant_values (experiment_id uuid REFERENCES variant_experiments ON DELETE CASCADE,
  arm int, key text, locale text, value text, PRIMARY KEY (experiment_id, arm, key, locale));
CREATE TABLE variant_counts (experiment_id uuid REFERENCES variant_experiments ON DELETE CASCADE,
  cell text, arm int, day date, views bigint DEFAULT 0, conversions bigint DEFAULT 0,
  revenue double precision DEFAULT 0, revenue_sq double precision DEFAULT 0,
  weight double precision, PRIMARY KEY (experiment_id, cell, arm, day));
ALTER TABLE projects ADD xp_holdout_pct int NOT NULL DEFAULT 0, ADD xp_persistence text NOT NULL DEFAULT 'none';
```
Les anciennes expériences (1 clé, 1 locale, `variant_arms`) restent servies par le chemin actuel.

### v2
Persistance `session` + precompute des pages statiques ; MCP/CLI ; tranches exploratoires BH ; revenu serveur (arms passés en metadata Stripe Checkout) ; `glot.experiments.toml` + export des personnalisations ; plan gating (segments = Pro+).

### À réutiliser
`allocate`, `posterior`, `decide`, `next_look`, `evaluate` (verrou de look atomique) ; `keys::check`, `measure::check`, `glossary::load`, `keys::write_translation` ; `insights::{country, required_per_arm, wilson}`, rate limit et `billing::consume(INSIGHTS|CDN)` ; `withVariants` ; `route()`, `createOnDemandLoader` (cache/ETag/SWR à copier pour la config) ; `PosteriorChart`, `ArmsTable`, page `/preview`.

### Risques
Cache HTML empoisonné (oubli de `private`) ; explosion des cellules (borne 4) ; trafic insuffisant (fusion auto) ; bras falsifiés par le client (déjà vrai aujourd'hui, impact limité aux compteurs : plafonner par session) ; personnalisations orphelines (re-test 90 j) ; interprétation juridique de l'exemption cookie (côté client).
