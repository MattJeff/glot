# Live review — contrat partagé (RV1…RV6)

> Propriétaire : RV1 (serveur). Les autres agents codent contre ce document ; toute évolution passe par le
> **Changelog** en bas (ajout = compatible ; jamais de renommage silencieux).
> Code : `crates/server/src/review_live/{mod,model,api,realtime}.rs`, migration `crates/server/migrations/0120_review.sql`,
> tests `crates/server/tests/review_live.rs`.

Besoin : le widget est sur le site du client ; dans le SaaS (page Review) on voit le site (iframe), on annote,
CRUD, fils de discussion, reformulation IA, appliquer une suggestion → la traduction validée est écrite. Tout est
aussi faisable hors du SaaS (API token, CLI, MCP, liens de revue pour des relecteurs sans compte).

Conventions : JSON snake_case, erreurs `{"error":{"code","message"}}` (comme le reste de l'API), dates RFC 3339,
ids UUID v4.

## 1. Modèle

### Tables (`0120_review.sql`)
| Table | Rôle |
|---|---|
| `review_settings` | `project_id` PK, `origins text[]` : origines autorisées du widget (CORS strict) |
| `review_links` | jetons du widget : `kind` `link` (lien partageable, relecteur sans compte) ou `member` (session widget d'un membre, 12 h). `token_hash` (sha256), `access` `read`\|`comment`\|`edit`\|`member`, `label`, `expires_at`, `revoked_at`, `created_by`, `user_id` (member) |
| `review_annotations` | annotation = ancre + message d'ouverture + statut + assignation + suggestion ; `key_id` (nullable), `feedback_group_id` (nullable, conversion d'un signalement) |
| `review_comments` | fil de réponses (markdown simple, `mentions uuid[]`) |
| `review_reactions` | réactions (annotation ou commentaire), `actor` unique par emoji |
| `review_attachments` | captures recadrées (PNG/JPEG/WebP ≤ 1 Mio, `bytea` en base comme `visual_files`) |
| `review_events` | historique append-only ; chaque ligne est aussi poussée en temps réel (NOTIFY `glot_review`) |
| `review_rate` | rate limit partagé Postgres (même schéma que `feedback_rate`) |

### Statuts
`open` (défaut) → `in_progress` → `resolved` | `wontfix` (toute transition permise ; `resolved_at` posé/effacé).

### Ancre (`Anchor`) — produite par le widget (RV2), stockée telle quelle (≤ 8 Ko), normalisée par le serveur
```ts
type Anchor = {
  key?: string | null;          // clé glot si connue
  key_source?: "attribute" | "marker" | "text" | "manual" | null;
                                // attribute = [data-glot-key] (closest), marker = marqueur dev @glot/runtime/dev,
                                // text = texte→clé côté client, manual = choisie par l'humain
  selector: string;             // sélecteur CSS robuste (≤ 1000) : #id stable > [data-testid] > [data-glot-key] >
                                // chemin tag:nth-of-type depuis l'ancêtre stable le plus proche
  text: string;                 // texte rendu de l'élément (≤ 1000, espaces normalisés)
  url: string;                  // URL de la page ; le serveur retire query + fragment
  route?: string;               // IGNORÉ en entrée : calculé par le serveur (voir ci-dessous)
  locale: string;               // locale affichée (BCP 47)
  viewport: { w: number; h: number; dpr?: number };   // px CSS
  rect?: { x: number; y: number; w: number; h: number }; // boîte de l'élément, px CSS relatifs au document
  point?: { x: number; y: number };   // position de l'épingle DANS l'élément, fractions 0..1 (défaut 0.5/0.5)
  dom?: string;                 // chemin de balises (≤ 300), comme feedback
};
```
Normalisation serveur : `url` sans `?…`/`#…` (≤ 500) ; `route` = chemin de `url`, sans slash final, segment de
locale initial retiré s'il vaut la locale (`/fr/pricing` → `/pricing`, `/fr` → `/`) ; `locale` validée.
Résolution de clé : `anchor.key` si elle existe dans le projet, sinon index inverse du catalogue de la locale sur
`anchor.text` (`feedback::match_key`, ICU = joker) ; sinon `key = null` (annotation libre, toujours acceptée).
Ré-ancrage côté client (RV2) : `[data-glot-key=key]` dont le texte ≈ `text` > `selector` > recherche de `text` > `rect`.

### Annotation (réponse)
```ts
type Actor = { user_id: string | null; link_id: string | null; name: string };
type Annotation = {
  id: string; status: "open" | "in_progress" | "resolved" | "wontfix";
  anchor: Anchor;                    // normalisée
  route: string; locale: string; url: string;
  key: string | null;                // clé résolue
  source: string | null;             // texte source de la clé
  translation: { value: string; status: string } | null;  // traduction courante (locale de l'annotation)
  body: string; body_html: string;   // markdown brut + HTML sûr rendu par le serveur
  suggestion: string | null;         // texte proposé pour remplacer la traduction
  author: Actor;
  assignee: { user_id: string; name: string } | null;
  feedback_group_id: string | null;
  comment_count: number;
  attachments: AttachmentMeta[];
  reactions: Reaction[];             // agrégées
  approvals: number;                 // = réactions "approve"
  created_at: string; updated_at: string; resolved_at: string | null;
  comments?: Comment[];              // seulement sur GET /annotations/{id}
};
type Comment = { id; annotation_id; author: Actor; body; body_html; mentions: string[]; reactions: Reaction[]; created_at; edited_at: string|null };
type Reaction = { emoji: "+1"|"-1"|"heart"|"eyes"|"check"|"approve"; count: number; mine: boolean };
type AttachmentMeta = { id; content_type; size; width: number|null; height: number|null; created_at };
type ReviewEvent = { id: number; annotation_id: string|null; comment_id: string|null; kind: string; actor: Actor; data: object; created_at };
```
Markdown accepté (`body`, ≤ 5000 car.) : `**gras**`, `*italique*`, `` `code` ``, `[texte](https://…)` (http/https
uniquement, `rel="noopener noreferrer nofollow" target="_blank"`), sauts de ligne. Tout le reste est échappé.
**Les clients affichent `body_html` (sûr) ou `body` en `textContent`, jamais `body` en innerHTML.** Caractères de
contrôle retirés à l'entrée. Mentions : `mentions: [user_id]` envoyé par le client (autocomplétion via
`GET …/members`), filtré aux membres du projet ; le texte contient librement `@Nom`.

## 2. Authentification

| Mode | Qui | Comment |
|---|---|---|
| Membre (SaaS, CLI, MCP) | membre du projet (tout rôle) | cookie `glot_session` ou `Authorization: Bearer glot_…` sur `/api/projects/{slug}/review/…` |
| Widget — lien de revue | relecteur sans compte | `Authorization: Bearer glr_…` (ou `?token=glr_…` pour SSE et `<img>`) sur `/api/review/w/…` |
| Widget — session membre | membre dans la page Review du SaaS ou sur son site | `POST /api/projects/{slug}/review/session` → `{token: "glr_…", expires_at}` (12 h) puis comme ci-dessus |

Jetons `glr_` : 32 octets aléatoires, seul le sha256 est stocké ; refusés si `revoked_at` ou `expires_at` passé
(401 `invalid_review_token`) ; un jeton `member` est aussi refusé si l'utilisateur n'est plus membre.
Lien partageable : `{web_or_site_url}?glot_review=<token>` — le widget lit `glot_review` dans l'URL, le garde en
`sessionStorage` et ne s'active que s'il en a un (zéro coût pour les visiteurs).

Droits :
| Action | `read` | `comment` | `edit` | `member` / membre |
|---|---|---|---|---|
| lire annotations, commentaires, captures, SSE, textes (§6) | ✓ | ✓ | ✓ | ✓ |
| créer annotation / commentaire / réaction / capture | | ✓ | ✓ | ✓ |
| modifier/supprimer **son** contenu (même lien + même nom) | | ✓ | ✓ | ✓ (auteur) |
| statut, assignation, suggestion d'autrui, appliquer une suggestion | | | ✓ | ✓ |
| édition manuelle du texte (§6) : écrire, annuler, supprimer une traduction, créer une clé | | | ✓ | ✓ |
| supprimer le contenu d'autrui | | | | admin |
| réglages, liens, conversion feedback | | | | admin (liens/réglages), membre (conversion) |

Relecteur par lien : chaque écriture porte `author_name` (1–80 car., requis pour `link`, ignoré sinon).

Sécurité widget (`/api/review/w/…`) : CORS strict — `Access-Control-Allow-Origin` uniquement pour une origine
déclarée dans `review_settings.origins` d'un projet (ou `GLOT_WEB_ORIGIN`), sans credentials ; en plus, chaque
requête portant `Origin` doit appartenir aux origines **du projet du jeton** (sinon 403 `origin_not_allowed`).
Rate limit partagé Postgres sur les écritures par jeton : `GLOT_REVIEW_RATE_PER_MIN` (défaut 120) par projet → 429
`rate_limited` + `Retry-After`. Corps ≤ 1 Mio + 64 Ko (captures ≤ 1 Mio, 413 `too_large`).

Billing (cloud) : chaque annotation créée compte `review_annotations` (mensuel, propriétaire du projet) → 402
`quota_exceeded` / `plan_required`. Plans : free 200, pro 10 000, team illimité (`GLOT_PLAN_LIMITS` surcharge).

## 3. Routes REST

Préfixes : **M** = `/api/projects/{slug}/review` (membre), **W** = `/api/review/w` (jeton widget).
Sauf mention, chaque route existe sous les deux préfixes avec le même corps et la même réponse.

```
GET    W /context               -> {project:{slug,name,source_locale,target_locales}, access:"read"|"comment"|"edit"|"member",
                                     me: Actor, expires_at|null}                         (W uniquement)
GET    M /settings              -> {origins:[string]}            PUT M /settings {origins} (admin ; "https://x.y[:port]", ≤ 20)
GET    M /links                 -> [ReviewLink]                  (admin)
POST   M /links {label?, access:"read"|"comment"|"edit", expires_in_days?: 1..365 (défaut 14, null = jamais)}
                                -> 201 {token, link: ReviewLink} (token montré une fois ; admin)
DELETE M /links/{id}            -> 204 (révocation immédiate ; admin)
ReviewLink = {id,label,access,created_at,expires_at|null,revoked_at|null,last_used_at|null,created_by:{user_id,name}}
POST   M /session               -> {token, expires_at}           (jeton widget membre, 12 h)
GET      /members               -> [{user_id,name}]              (mentions + assignation)

GET      /annotations?route=&locale=&status=open,in_progress&key=&author=&assignee=&q=&page=1&per_page=50
                                -> {items:[Annotation], total}   (status = liste séparée par virgules ; author = user_id
                                   ou nom exact ; assignee = user_id ou "me" ; q = texte du corps/ancre ; tri updated_at desc)
POST     /annotations {anchor: Anchor, body, suggestion?, assignee_id?, author_name?} -> 201 Annotation
GET      /annotations/{id}      -> Annotation (+ comments)
PATCH    /annotations/{id} {status?, assignee_id?: uuid|null, body?, suggestion?: string|null, anchor?} -> Annotation
DELETE   /annotations/{id}      -> 204
GET      /annotations/{id}/history -> [ReviewEvent]  (ancien → récent)

POST     /annotations/{id}/comments {body, mentions?:[user_id], author_name?} -> 201 Comment
PATCH    /annotations/{id}/comments/{cid} {body, mentions?} -> Comment       (auteur)
DELETE   /annotations/{id}/comments/{cid} -> 204                              (auteur ou admin)

POST     /annotations/{id}/reactions {emoji, comment_id?, author_name?} -> {reactions:[Reaction]}  (bascule on/off)

POST     /annotations/{id}/attachments   corps brut image/png|jpeg|webp (≤ 1 Mio, octets magiques vérifiés)
                                -> 201 AttachmentMeta   (max 5 par annotation ; ?author_name= pour un lien)
GET      /annotations/{id}/attachments/{aid} -> image  (Cache-Control private, immutable ; nosniff)
DELETE   /annotations/{id}/attachments/{aid} -> 204

POST     /annotations/{id}/apply {value?} -> Annotation
           Écrit `value` (défaut : suggestion) comme traduction validée de (key, locale) via `keys::write_translation`
           (origin=human, status=reviewed, auteur = acteur) ; 422 `invalid_translation` si le validateur trouve une
           erreur, 409 `no_key` si l'annotation n'a pas de clé, 400 si locale non cible. Passe l'annotation en
           `resolved` ; si `feedback_group_id` est ouvert/proposé, le groupe passe `accepted`.  (accès `edit` ou membre)

POST   M /from-feedback/{group_id} -> 201 Annotation   (convertit un groupe de signalements feedback en annotation :
           ancre = dernier signalement (url, dom, texte), clé + locale du groupe, suggestion = proposition IA sinon
           suggestion utilisateur, corps = résumé des signalements ; 409 si déjà converti)
GET      /events                -> SSE (voir §4)
```
Réservé à RV4 (reformulation IA) : `POST …/annotations/{id}/rephrase {instruction?}` sous M et W (W : accès
`member` ou `comment`). RV4 enregistre son résultat avec `review_live::model::set_suggestion(state, project_id,
annotation_id, value, &actor)` (pose `suggestion`, journalise `suggestion_set`, pousse l'événement). Pas besoin de
toucher aux tables.

## 4. Temps réel (SSE)
`GET {M|W}/events` (`?token=` accepté sous W ; `Last-Event-ID` ou `?since=<id>` rejoue jusqu'à 500 événements
manqués). Un message par ligne de `review_events` du projet :
```
event: review
id: <ReviewEvent.id>
data: ReviewEvent
```
`kind` ∈ `annotation.created` | `annotation.updated` (data = champs changés : `{status?, assignee_id?, body?,
suggestion?, anchor?}`) | `annotation.deleted` | `comment.created` | `comment.updated` | `comment.deleted` |
`reaction.toggled` (`{emoji, comment_id, on}`) | `attachment.created` | `attachment.deleted` |
`suggestion.applied` (`{key, locale, value}`) | `suggestion_set` (RV4) ; une conversion feedback émet `annotation.created` avec `{feedback_group_id}`.
Les clients refetch `GET /annotations/{id}` à la réception (le payload reste < 8 Ko). Keep-alive commentaire toutes
les 15 s. Multi-réplicas : `pg_notify('glot_review', …)` dans la transaction d'écriture, un `LISTEN` par process.

## 5. Protocole iframe (RV2 widget ↔ RV3 page Review)
`window.postMessage`, cible = origine connue (page : origine du site ; widget : `GLOT_WEB_ORIGIN` passé par
`data-app-origin`). Widget → page : `glot:review:ready {url, route, locale}`, `glot:review:navigated {url, route,
locale}`, `glot:review:selected {annotation_id}`, `glot:review:created {annotation_id}`. Page → widget :
`glot:review:focus {annotation_id}`, `glot:review:mode {annotate: boolean}`, `glot:review:filter {status?: string[]}`.
La page Review mint le jeton par `POST M/session` et charge l'iframe `site_url?glot_review=<token>`.
Si l'appel `context` du widget échoue : `glot:review:error {status}` (401 = jeton, 0 = réseau/CORS).
Diagnostic côté serveur : `GET M/probe` récupère la `base_url` du projet (IP publiques uniquement, 6 s, 512 Ko) →
`{status, error, x_frame_options, frame_ancestors, framing: allowed|blocked|unknown, widget, widget_project,
widget_app_origin, feedback_widget}`.

## 6. Édition manuelle du texte (sans IA)
Même préfixes M/W, mêmes jetons. Lecture : tout accès. Écriture : membre (tout rôle) ou lien `edit`.
Toute écriture passe par `keys::write_translation` (validateur ICU + glossaire, `origin=human`, `status=reviewed`,
`translation_history` avec l'auteur = acteur) et journalise un `ReviewEvent` (`text.*`, SSE inclus).
`{key}` est encodé URL (`encodeURIComponent`).
```
GET    /text/{key}          -> TextEntry
TextEntry = {key, source, source_locale, description|null, ui_role|null,
             values: {<locale>: Translation}}       // Translation = celle de l'API keys (status "missing" si absente)
PUT    /text/{key} {values: {<locale>: string}, fit?: Fit, dry_run?: bool, author_name?}
         -> 200 {results: {<locale>: {issues: Issue[], translation: Translation|null}}}
         Une ou plusieurs langues d'un coup, ATOMIQUE : si une langue a une erreur (validateur, glossaire, budget
         pixels) → 422 `invalid_translation` avec le même `results` dans `error.details`, rien n'est écrit.
         dry_run=true → valide sans écrire (validation en direct pendant la frappe ; 200 même avec erreurs).
         `fit` = budget pixels mesuré par le widget sur l'élément (format `glot_core::measure::Fit` :
         {max_width_px, max_lines|null, font:{family,size_px,weight,letter_spacing_px,text_transform?},
          text?, text_px?}) → `too_wide` (erreur au-delà du budget, warning dans la marge de 5 %).
         Locales = cibles du projet (400 sinon). Valeur vide refusée (utiliser DELETE).
GET    /text/{key}/history?locale= -> [{id, locale, value, origin, model|null, author|null, created_at}] (récent d'abord, ≤ 200)
POST   /text/{key}/revert {locale, history_id?, author_name?} -> {translation: Translation}
         Réécrit une version antérieure (défaut : la version précédant la valeur courante) ; revalidée ; 409
         `nothing_to_revert` s'il n'y en a pas. L'annulation est elle-même une nouvelle version (ré-annulable).
DELETE /text/{key}/translations/{locale} -> 204
         Supprime la traduction de la locale (le runtime retombe sur le fallback/source) ; l'historique est conservé,
         donc `revert` la restaure.
POST   /text {source, key?, description?, anchor?: Anchor, values?: {<locale>: string}, author_name?} -> 201 TextEntry
         Crée une clé à partir d'un texte en dur annoté (= add_source). `key` absent → générée depuis la route et le
         texte (`pricing.start_free_trial`, suffixe `_2`… si prise) ; 409 si `key` fournie existe ; `values`
         optionnelles écrites comme ci-dessus. Le remplacement du littéral dans le code reste côté dev (RV5 : CLI/MCP
         `glot i18nize`, PR GitHub).
```
Événements SSE : `text.updated` (`{key, locales:[…]}`), `text.reverted` (`{key, locale, history_id}`),
`text.deleted` (`{key, locale}`), `text.created` (`{key}`) ; `annotation_id` = null.

## Changelog
- 2026-10-18 — v1 (RV1) : tables, ancre, statuts, auth (liens `read`/`comment`, session membre), routes M/W, SSE,
  protocole iframe, route `rephrase` réservée à RV4, quota `review_annotations`.
- 2026-10-18 — v1.1 (RV1, demande fondateur « CRUD du texte sans IA ») : §6 édition manuelle (lecture toutes langues,
  écriture multi-langues atomique + dry_run, budget pixels `fit`, historique, revert, suppression → fallback,
  création de clé depuis un texte en dur) ; nouvel accès de lien `edit` ; « appliquer une suggestion » et statuts
  ouverts aux liens `edit`.
- 2026-10-18 — v1.2 (RV3, ajout compatible au §5, pour l'édition manuelle + aperçu en direct) : page → widget
  `glot:review:mode {annotate, pick?: boolean}` (pick = le prochain clic sur un texte n'ouvre pas le composeur du
  widget mais renvoie `glot:review:picked {anchor: Anchor}` à la page, Échap → `glot:pick-cancel`) et
  `glot:review:preview {annotation_id: string|null, anchor: {key?, selector, text}|null, value: string|null}`
  (remplace temporairement le texte de l'élément ancré par `value` ; `null` restaure). La page Review envoie aussi
  l'équivalent du script bridge `web/lib/preview-bridge.ts` (`glot:anchors`/`glot:rects`, `glot:pick`/`glot:picked`,
  `glot:replace`, `glot:scroll`) pour les sites sans widget (positions des épingles dessinées par la page).
- 2026-10-18 — v1.2 (RV4, ajout compatible) : `POST {M|W}/annotations/{id}/rephrase {preset?, segment?, instruction?, count?, fit?,
  refresh?, author_name?}` (accès `comment`+ ; défaut sans preset ni instruction = corps de l'annotation) → `{rewrite_id, cached,
  created_at, key, locale, proposals:[{id, text, explanation, valid, variants:{<locale>:{value, back_translation, issues, policy,
  valid}}}], rejected:[{text, reason}], usage:{input_tokens, output_tokens, cost_usd…}, model}` ; la 1re proposition valide devient
  la `suggestion` (`set_suggestion`). Cache `review_rewrites` (migration `0121_review_rewrite.sql`) par hash (clé, instruction,
  contexte). `POST {M|W}/annotations/{id}/rephrase/apply {rewrite_id, proposal_id, author_name?}` (accès `edit`+) → écrit toutes
  les langues cibles de la proposition comme `PUT /text` (atomique, 422 `invalid_translation`), résout l'annotation, SSE
  `suggestion.applied` ; 409 `invalid_proposal` si une variante est invalide ; réponse `{annotation, results, source_text}`.
  Code : `review_live/rewrite.rs` ; `api::{check_values, write_values, results_json, invalid}` passés `pub(super)` pour ce partage.
- 2026-10-18 — v1.3 (RV1, implémenté + testé `tests/review_live.rs`) : `Comment.reactions` ajouté ; la conversion feedback émet
  `annotation.created` (data `feedback_group_id`) au lieu de `feedback.converted` ; limite de corps = 1 Mio + 64 Ko ; pour un lien,
  DELETE et upload de capture prennent le nom via `?author_name=` ; `GET /api/review/w/context` aussi servi sous W uniquement.
  Quota billing `review_annotations` : free 200/mois, pro 10 000, team illimité.
- 2026-10-18 — v1.4 (RV5, ajout compatible, revue hors du SaaS) : `POST M/annotations/{id}/github {target: "issue"|
  "pr_comment"}` → `{kind, number, url, created}` (idempotent ; 409 `github_not_linked` / `no_glot_pr`) — miroir en
  issue du repo lié ou en commentaire sur la PR glot ouverte (table `review_github`, migration `0125_review_github.sql`,
  événement `github.mirrored {kind, number, url}`). Webhook `pull_request` fermé **et mergé** : les annotations
  ouvertes miroir de cette PR (`pr_comment`) ou d'une issue qu'elle ferme (`Fixes #12`…) passent `resolved` avec un
  commentaire « Fixed by #n » de l'acteur `{name: "GitHub"}` (`comment.created` + `annotation.updated`). La GitHub
  App demande maintenant `issues: write` (installations existantes : accepter la permission). `api::Ids::of` pour
  les appels in-process (MCP distant). CLI `glot annotations …`, outils MCP `annotations_list`, `annotation_reply`,
  `annotation_resolve`, `annotation_apply_suggestion`, `annotation_rewrite` (local + distant), code de check
  `open_annotation` (warning).
