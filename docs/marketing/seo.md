# SEO glot : audit, corrections, architecture, mots-clés, plan de contenu

Audit fait le **2026-10-18** sur https://glotlang.com (prod) et sur le repo (`web/`). Aucune modification de code n'a été faite : chaque correction indique le fichier et le changement à faire.

> **La prod est en retard sur le repo.** `/pricing` et `/docs/experiments` renvoient 404 en ligne, `/docs` fait une 307 vers `/docs/getting-started`, et le `<head>` en ligne n'a ni canonical, ni hreflang, ni Open Graph, alors que `web/app/[locale]/page.tsx` et `docs/[slug]/page.tsx` les déclarent. **Correction n°0 : redéployer**, puis refaire les vérifications de la section 1.4. Les constats ci-dessous distinguent **[prod]** (observé en ligne) et **[repo]** (encore présent dans le code).

---

## 1. Audit du site actuel

### 1.1 Constats observés (curl sur la prod, 2026-10-18)

| URL | Statut | Constat |
|---|---|---|
| `/` | 200, 201 KB HTML, TTFB ~0,28 s, `x-nextjs-cache: HIT` | Title `glot: your agent localizes, code guarantees` (44 car.), meta description OK. **Ni canonical, ni og:\*, ni twitter:\*, ni JSON-LD** dans le HTML [prod]. hreflang présent **uniquement dans l'en-tête HTTP `Link`** (ajouté par le middleware next-intl, avec `x-default`). Aucun lien vers `/docs` dans le HTML [prod]. |
| `/fr`, `/ja` | 200 | Landing traduite, `lang` correct, `dir="rtl"` pour `ar`. |
| `/docs` | **307** → `/docs/getting-started` [prod] | Redirection temporaire. Le repo a désormais une vraie page d'accueil des docs. |
| `/fr/docs/getting-started` | 200 | `<html lang="fr">`, title en français, **corps en anglais**, et **meta description = celle de la landing** [prod]. Le H1 contient un `<a>` d'ancre (sans conséquence). |
| `/pricing`, `/docs/experiments` | **404** [prod] | Pages présentes dans le repo, pas déployées. |
| `/robots.txt`, `/sitemap.xml`, `/llms.txt` | **404** [prod + repo] | Aucun fichier `robots.ts` / `sitemap.ts` dans `web/app`. |
| `/opengraph-image` | 404 ; aucune `og:image` dans le HTML [prod] | `web/app/[locale]/opengraph-image.tsx` existe dans le repo, donc à vérifier après le redéploiement. |
| `/en` | **307** → `/` | Une redirection permanente (308) serait préférable pour consolider. |
| `/login`, `/signup` | 302 → `app.glotlang.com` | Normal. |
| `http://`, `www.` | 308 → `https://glotlang.com/` | OK. |
| `/docs/getting-started/` | 308 → sans slash | OK. |
| 404 | vrai 404 (pas de soft 404) | OK. |
| **`app.glotlang.com/docs/getting-started`, `app.glotlang.com/fr`** | **200** | **Le site marketing entier est dupliqué sur le sous-domaine app**, sans canonical ni noindex [prod]. `app.glotlang.com/login` est indexable avec le title de la landing. `app.glotlang.com/robots.txt` renvoie 404. |
| `api.glotlang.com/` | 404 | OK. |

Performance perçue : HTML servi depuis le cache (s-maxage 1 an), TTFB entre 0,2 et 0,3 s, 13 chunks JS (~270 KB décodés), 2 polices préchargées. La vidéo du hero a un `poster` et `preload="metadata"`, le WASM du playground (269 KB) n'est chargé qu'à l'interaction : c'est bien. Points faibles : 3 PNG de 220 à 250 KB dans `public/landing/` (`inspect.png`, `popover-*.png`), et des GIF de 2,5 à 5,5 MB dans `public/launch/`, à ne jamais référencer depuis une page. L'API PageSpeed avait épuisé son quota et Chrome DevTools était occupé : il n'y a **pas encore de mesure Lighthouse ni de données terrain (CrUX)**. À faire après le redéploiement (section 1.4).

### 1.2 Problèmes dans le code [repo]

1. **Pas de `robots.ts` ni de `sitemap.ts`.**
2. **Le contenu des docs n'existe qu'en anglais mais il est servi sous 6 locales**, avec un canonical auto-référent et des hreflang vers chaque locale (`docs/[slug]/page.tsx`). Google traite comme des doublons les pages dont seul le gabarit est traduit ; les hreflang qui pointent vers un corps identique sont du bruit et diluent le signal.
3. **La page d'accueil des docs et la page tarifs n'ont ni `alternates` ni `openGraph`** (`docs/page.tsx`, `pricing/page.tsx`).
4. **Le title de la page tarifs est doublé** : `messages/en.json` → `pricing.meta.title = "Pricing · glot"` et le layout applique le gabarit `%s · glot`, ce qui donne `Pricing · glot · glot`.
5. **`pricing` (et 10 autres namespaces) n'existe pas dans `fr/es/de/ja/ar.json`** : `/fr/pricing` sert de l'anglais ou un repli, donc un doublon. Pour un outil i18n qui pratique le dogfooding, c'est aussi un problème de crédibilité.
6. **Pas de `x-default` dans `alternates.languages`** (page.tsx, docs/[slug]). Le middleware l'ajoute en en-tête HTTP, mais les deux signaux doivent être identiques et présents dans le HTML.
7. **`openGraph.locale` reçoit `fr`** au lieu de `fr_FR` (format attendu par OG).
8. **Aucune donnée structurée** (Organization, WebSite, SoftwareApplication, FAQPage, BreadcrumbList, TechArticle).
9. **L'image OG est uniquement en anglais** (choix assumé dans `opengraph-image.tsx`). C'est acceptable, mais elle s'applique aussi aux docs, qui mériteraient leur propre image avec le titre de la page.
10. **Le sous-domaine app sert tout le site Next** (`deploy/Caddyfile`, bloc `app.{$GLOT_DOMAIN}`).

### 1.3 Corrections priorisées

| Prio | Fichier | Changement |
|---|---|---|
| **P0** | (déploiement) | Redéployer `web/` : la prod n'a ni canonical, ni hreflang HTML, ni OG, ni /pricing. |
| **P0** | `deploy/Caddyfile`, bloc `app.{$GLOT_DOMAIN}` | Ajouter `header X-Robots-Tag "noindex, nofollow"` et rediriger tout ce qui ne relève pas du dashboard vers l'apex : `@marketing not path_regexp ^(/(en\|fr\|es\|de\|ja\|ar))?/(app\|login\|signup\|oauth\|forgot-password\|reset-password\|verify-email\|admin\|api\|cdn\|_next)(/\|$)` → `redir @marketing https://{$GLOT_DOMAIN}{uri} 308`. |
| **P0** | `web/app/robots.ts` (nouveau) | `rules: { userAgent: "*", allow: "/", disallow: ["/api/", "/cdn/", "/*/app", "/app"] }`, `sitemap: "https://glotlang.com/sitemap.xml"`, `host` facultatif. Ne pas bloquer les bots IA (GPTBot, ClaudeBot, PerplexityBot) : pour un devtool, être cité dans les réponses génératives est un canal d'acquisition. |
| **P0** | `web/app/sitemap.ts` (nouveau) | Une entrée par page marketing × 6 locales, avec `alternates.languages` (dont `x-default`). Pour les docs, **seulement les URL anglaises** (cf. P1 docs). Réutiliser `routing.locales` et `SLUGS` (`docs/content.ts`) ; `lastModified` = mtime du `.md`. |
| **P0** | `web/messages/en.json` | `pricing.meta.title` → `"Pricing"` (le gabarit ajoute ` · glot`). |
| **P1** | `web/app/[locale]/docs/[slug]/page.tsx` | Tant que les docs sont en anglais : `canonical` = URL **anglaise** quelle que soit la locale, et retirer `languages` (ou mettre `robots: { index: false, follow: true }` pour `locale !== "en"`). Ajouter un JSON-LD `TechArticle` + `BreadcrumbList`. Garder `lang="en"` sur `<article>`. |
| **P1** | `web/app/[locale]/docs/page.tsx` | Ajouter `alternates` (même règle que ci-dessus) et `openGraph`. Écrire une description dédiée (`docs.start.subtitle` est trop générique). |
| **P1** | `web/app/[locale]/pricing/page.tsx` | Ajouter `alternates` (canonical + 6 langues + `x-default`) et `openGraph`. |
| **P1** | `web/messages/{fr,es,de,ja,ar}.json` | Traduire au moins `pricing` (via `glot translate`). Sinon, `noindex` les locales non traduites. |
| **P1** | `web/app/[locale]/page.tsx` + `pricing` + `docs` | Ajouter `"x-default": SITE` dans `languages`. Passer `openGraph.locale` par une table (`en_US, fr_FR, es_ES, de_DE, ja_JP, ar_AR`) et ajouter `alternateLocale`. |
| **P1** | `web/app/[locale]/layout.tsx` | Injecter un `<script type="application/ld+json">` avec `Organization` (name, url, logo, sameAs: GitHub, npm) et `WebSite`. |
| **P1** | `web/app/[locale]/page.tsx` | JSON-LD `SoftwareApplication` (cf. §5.4), plus `FAQPage` construit à partir des questions **visibles** du composant `Faq`. Google ne montre plus de résultats enrichis FAQ depuis le 7 mai 2026, mais continue de parser le balisage, et les moteurs génératifs aussi. |
| **P1** | `web/proxy.ts` / next-intl | `/en` → 308 plutôt que 307 (`localeDetection`/redirection permanente ou règle Caddy `redir /en / 308`, `redir /en/* /{path} 308`). |
| **P2** | `web/app/[locale]/docs/[slug]/opengraph-image.tsx` (nouveau) | Image OG par page de doc, avec le titre de la page (même gabarit que la landing). |
| **P2** | `web/public/landing/*.png` | Convertir en AVIF/WebP (next/image le fait déjà à la volée si `<Image>` est utilisé partout ; vérifier qu'aucun `<img>` brut ne les charge). Vérifier que les GIF de `public/launch/` ne servent qu'au README. |
| **P2** | `web/app/llms.txt/route.ts` (nouveau) | `llms.txt` qui liste les docs (titre + URL `.md` brute sur GitHub). L'effet mesuré est faible (aucune hausse de citations démontrée, Google l'ignore), mais le coût est de ~20 lignes et c'est une convention répandue chez les devtools. Priorité basse. |
| **P2** | `web/content/docs/*.md` | Chaque doc devrait ouvrir sur une phrase descriptive (elle sert de meta description, tronquée à 200 caractères ; viser 140 à 160). |
| **P2** | Search Console | Valider le domaine (propriété de domaine, qui couvre `app.` et `api.`), soumettre le sitemap, surveiller « Pages en double sans canonical », et le rapport INP terrain. |

### 1.4 Vérifications après le redéploiement

```sh
curl -s https://glotlang.com/ | grep -Eo '<link rel="(canonical|alternate)"[^>]*>|og:[a-z:]+|application/ld\+json'
curl -s https://glotlang.com/sitemap.xml | head; curl -s https://glotlang.com/robots.txt
curl -sI https://app.glotlang.com/docs/getting-started | grep -i -E 'location|x-robots'
```
Lighthouse mobile sur `/`, `/docs/getting-started` et `/pricing`. Cibles : LCP < 2,5 s, CLS < 0,1, **INP < 200 ms** (le playground et le sélecteur de langue sont les interactions à surveiller).

---

## 2. Bonnes pratiques 2026 retenues (devtool, Next.js App Router, 6 langues)

- **Core Web Vitals** : l'INP remplace le FID depuis mars 2024 (seuils : bon < 200 ms, mauvais > 500 ms, au 75e percentile terrain). Les CWV restent un facteur de départage entre pages déjà pertinentes, pas un levier de classement majeur. Les annonces d'une pondération renforcée en 2026 ne sont pas confirmées par Google. Rester en rendu serveur/statique (déjà le cas), garder les composants client petits et charger le WASM et la vidéo à la demande (déjà le cas).
- **hreflang** : à déclarer dans `alternates.languages` avec `x-default`, de façon réciproque et identique entre le HTML, le sitemap et l'en-tête `Link`. **Uniquement pour des pages dont le contenu principal est réellement traduit.**
- **Sitemap** : utiliser le `app/sitemap.ts` natif, avec des alternates par entrée, générés à partir des mêmes listes que `generateStaticParams` (pas d'entrée qui pointe vers un 404).
- **Données structurées** : les résultats enrichis FAQ ne s'affichent plus depuis le 7 mai 2026, et les HowTo ont été retirés en 2023. Le balisage reste parsé : le garder seulement s'il décrit un contenu visible. Prioriser `Organization`, `WebSite`, `SoftwareApplication`, `BreadcrumbList` et `TechArticle`.
- **Moteurs génératifs (AI Overviews, ChatGPT, Claude, Perplexity)** : pour un devtool, ce sont des pages factuelles, datées et sourcées qui se font citer. Tableaux comparatifs avec sources (`docs/comparison-sources.md` est déjà un atout), commandes copiables, chiffres de benchmark avec méthode. `llms.txt` : adoption de 24 à 36 % dans la tech, effet mesuré nul pour l'instant.
- **Pages programmatiques** : une page par framework uniquement si elle contient un vrai contenu propre (commande, fichiers écrits, exemple vérifié). Le repo a `examples/frameworks/*` et `examples/mobile-formats/*`, ce qui justifie ces pages et protège contre le « scaled content abuse ».

---

## 3. Architecture de pages cible

```
/                                  Nouvelle landing : expériences de copy par segment, multi-langues (6 locales)
/i18n                              Ex-landing : « localisation open source pour agents de code » (6 locales)
/pricing                           (6 locales, à traduire)
/compare/                          Hub des comparatifs (EN, puis FR/DE)
  /compare/lokalise   /compare/crowdin   /compare/lingo-dev   /compare/tolgee   /compare/phrase
/i18n/{framework}                  Pages programmatiques web : nextjs, react, vite, react-router, astro
/i18n/ai-builders/{tool}           lovable, bolt, v0, replit
/localize/{platform}               ios-swiftui (xcstrings), android, flutter (ARB), rails, django
/glossary/{terme}                  i18n, l10n, cldr-plurals, icu-messageformat, pseudo-localization,
                                   multilingual-copy-testing, multi-armed-bandit, holdout-group
/docs/...                          EN uniquement, canonical EN (existant)
/blog/{slug}                       Articles (EN, plus FR si pertinent)
```

**Maillage interne** :
- Landing → `/docs/experiments` (installation en 3 lignes), `/docs/insights`, `/i18n`, `/compare`.
- `/i18n` → `/docs/getting-started`, `/i18n/{framework}`, `/compare/*`.
- Chaque `/i18n/{framework}` → `/docs/frameworks#<ancre>` + `examples/frameworks/<x>` sur GitHub + `/i18n` (BreadcrumbList).
- Chaque page de docs → un bloc « Voir aussi » vers la page marketing correspondante (`experiments.md` → `/`, `frameworks.md` → `/i18n/nextjs`, etc.). Aujourd'hui les docs ne renvoient vers aucune page marketing.
- Glossaire → docs + landing ; les comparatifs → `/pricing`.
- Pied de page commun (landing, docs, pricing) avec les hubs : Docs, Compare, Frameworks, Glossary, GitHub.

**Langues** : landing, `/i18n` et `/pricing` en 6 langues, traduites avec glot (dogfooding, à afficher). Comparatifs et pages framework en EN d'abord, puis FR/DE/ES/JA si la Search Console montre des impressions. **Ne jamais publier une locale dont le corps n'est pas traduit.** L'arabe : uniquement landing et pricing (demande dev faible).

---

## 4. 30 mots-clés prioritaires

Volumes = **ordres de grandeur estimés** (recherches mensuelles mondiales, Google), sans accès à Keyword Planner, Ahrefs ou Semrush. À valider avant d'engager du contenu. Repères indirects : react-i18next ~8 à 10 M de téléchargements npm par semaine, next-intl ~1,5 M (2026). Intention : I = informationnelle, C = commerciale/comparaison, T = transactionnelle/outil.

| # | Mot-clé | Vol. estimé | Int. | Difficulté | Page cible |
|---|---|---|---|---|---|
| 1 | next.js i18n | 5–10k | I | Haute | `/i18n/nextjs` |
| 2 | next-intl | 5–10k | I | Haute (docs officielles) | `/i18n/nextjs` + blog |
| 3 | react i18n | 3–6k | I | Haute | `/i18n/react` |
| 4 | react-i18next tutorial | 1–3k | I | Moy. | `/i18n/react` |
| 5 | translate react app with ai | 200–800 | T | Faible | `/i18n` |
| 6 | ai localization tool | 500–1,5k | C | Moy. | `/i18n` |
| 7 | open source translation management | 300–1k | C | Moy. | `/i18n` |
| 8 | translation management system for developers | 200–600 | C | Moy. | `/i18n` |
| 9 | lokalise alternative | 300–1k | C | Moy. | `/compare/lokalise` |
| 10 | crowdin alternative | 200–800 | C | Moy. | `/compare/crowdin` |
| 11 | lingo.dev alternative / lingo.dev vs tolgee | 100–400 | C | Faible | `/compare/lingo-dev` |
| 12 | tolgee alternative | 100–300 | C | Faible | `/compare/tolgee` |
| 13 | phrase alternative localization | 100–400 | C | Moy. | `/compare/phrase` |
| 14 | localize lovable app / translate lovable website | 100–500 (en forte hausse) | T | Faible | `/i18n/ai-builders/lovable` |
| 15 | bolt.new multilingual / v0 i18n | 50–200 | T | Faible | `/i18n/ai-builders/bolt`, `/v0` |
| 16 | swiftui localization / xcstrings | 1–2k | I | Moy. | `/localize/ios-swiftui` |
| 17 | flutter localization arb | 1–3k | I | Moy. | `/localize/flutter` |
| 18 | android string localization | 500–1,5k | I | Moy. | `/localize/android` |
| 19 | icu message format plural | 300–1k | I | Faible | `/glossary/icu-messageformat` |
| 20 | cldr plural rules | 300–800 | I | Faible | `/glossary/cldr-plurals` |
| 21 | pseudo localization | 300–800 | I | Faible | `/glossary/pseudo-localization` |
| 22 | i18n mcp server / localization mcp | 100–300 (émergent) | T | Faible | `/docs/mcp` |
| 23 | i18n github action / translation ci | 100–400 | T | Faible | `/docs/integrations` |
| 24 | text overflow translation / ui text expansion | 100–300 | I | Faible | blog (pixel width) |
| 25 | multilingual copy testing | 50–200 | C | Faible | **Landing** |
| 26 | localized landing page optimization / multilingual conversion optimization | 100–400 | C | Moy. | **Landing** |
| 27 | personalize website copy by country / audience segment | 200–600 | C | Moy. | **Landing** |
| 28 | ai copy variations generator | 500–1,5k | T | Moy. | **Landing** + blog |
| 29 | a/b testing translations / a/b test localized content | 100–300 | I/C | Faible | `/glossary/multilingual-copy-testing` + `/docs/experiments` |
| 30 | multivariate testing vs a/b testing | 2–5k | I | Haute | blog (§6.8), lien vers la landing |

**Capter la demande « A/B testing » sans le mot sur la landing.** Les requêtes « a/b testing tool », « multivariate testing » et « ai a/b testing » ont un gros volume (10k+ pour la tête) mais sont dominées par Optimizely, VWO, Kameleoon et les listes « best tools », et l'intention est surtout marketing/CRO. Stratégie :
1. La landing vise le créneau libre : **copy multi-langue × segment × validation** (#25 à #28). C'est peu concurrentiel et ça décrit exactement le produit.
2. Le terme « A/B » apparaît **uniquement** dans le glossaire (`/glossary/multilingual-copy-testing`, H2 « Différence avec l'A/B testing classique »), dans la doc `/docs/experiments` (une phrase dans le paragraphe d'introduction, plus une section FAQ « Is this A/B testing? ») et dans 2 articles de blog (§6.8, §6.9). Ces pages font un lien vers la landing avec l'ancre « segmented copy experiments ».
3. Le **bandit** (« multi-armed bandit copy testing », « bandit vs a/b test ») est un angle technique adapté aux devs, que les outils CRO couvrent peu : un article de glossaire dédié.

---

## 5. Spécification SEO de la nouvelle landing (`/`)

### 5.1 Positionnement des requêtes

- **Principal** : multilingual copy experiments / localized copy testing per segment.
- **Secondaires** : AI-generated copy variants, validated in every language ; personalize copy by country/audience ; server-rendered, no flicker ; open source.
- **Marque** : « glot », « glot i18n », « glotlang ».

### 5.2 Title et meta (EN, puis à traduire avec glot)

- **Title** (≤ 60 car.) : `glot: copy experiments in every language, per segment`
  Variante : `Multilingual copy experiments, AI-written and validated · glot`
- **Meta description** (≤ 155 car.) : `Generate copy variants with AI, validated in every language (placeholders, plurals, glossary, length), and keep the one that converts for each audience.`
- **FR** : title `glot : expériences de copy dans chaque langue, par segment` ; description `Variantes de textes générées par IA et validées dans chaque langue (variables, pluriels, glossaire, longueur). Gardez celle qui convertit, segment par segment.`
- Les autres locales sont traduites par glot puis relues ; ne jamais réutiliser le title EN.
- `alternates.canonical` auto-référent, `languages` × 6 + `x-default` ; `openGraph` (type `website`, `locale` au format `xx_XX`, `alternateLocale`) ; `twitter.card = summary_large_image` ; nouvelle image OG qui montre une clé en 3 variantes × 3 langues, avec un segment et un badge « validated ».

### 5.3 Plan Hn

- **H1** : `Test your copy in every language, for every audience.`
  (FR : `Testez vos textes dans chaque langue, pour chaque audience.`)
- Sous-titre (p) : glot génère des variantes par intention, les valide comme des traductions et sert la bonne variante par segment, rendue côté serveur.
- **H2** `One idea, written in every language` : arms/intentions, générées par IA à partir d'une intention, validées (placeholders, CLDR, glossaire, budget de longueur).
- **H2** `Different audiences, different winners` : segments (pays, source, appareil), jusqu'à 4 segments plus `*`, apprentissage par cellule.
- **H2** `No flicker, no critical-path dependency` : décision à l'edge, rendu serveur, repli sur le texte officiel si glot est injoignable, `Cache-Control: private`.
- **H2** `Measure what converts, not what is clicked` : objectifs (signup, purchase + revenu), garde-fous (refund), holdout.
- **H2** `Install in three lines` : snippet `glotProxy` / `withExperiments` / `track` → lien `/docs/experiments`.
- **H2** `Built on glot's localization guarantees` → lien `/i18n` (ancre « open-source localization for coding agents »). Ce lien conserve la pertinence i18n que la home porte aujourd'hui.
- **H2** `FAQ` (visible) : Does it slow my pages? / Does it work without Next.js? (Cloudflare Worker, SPA) / How is a winner chosen? / Is my data shared? / Is it open source? / Which languages?
- Footer : hubs (cf. §3).

Contraintes : un seul H1 ; le terme « A/B » n'apparaît nulle part (texte, alt, title, JSON-LD) ; le vocabulaire est **experiment, variant, segment, intention, conversion**.

### 5.4 Données structurées (JSON-LD, dans `page.tsx`)

```json
[
  {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    "name": "glot",
    "url": "https://glotlang.com/",
    "applicationCategory": "DeveloperApplication",
    "operatingSystem": "macOS, Linux, Windows",
    "description": "<= meta description de la locale>",
    "inLanguage": "<locale>",
    "offers": { "@type": "Offer", "price": "0", "priceCurrency": "USD" },
    "license": "https://opensource.org/licenses/MIT",
    "softwareVersion": "<version npm, déjà récupérée par getRepoStats()>",
    "downloadUrl": "https://www.npmjs.com/package/glot-i18n",
    "sameAs": ["https://github.com/MattJeff/glot", "https://www.npmjs.com/package/glot-i18n"],
    "publisher": { "@id": "https://glotlang.com/#org" }
  },
  { "@context": "https://schema.org", "@type": "FAQPage", "mainEntity": ["<questions/réponses visibles, traduites>"] }
]
```
`Organization` (`@id: https://glotlang.com/#org`) et `WebSite` sont déclarés dans le layout. Pas d'`aggregateRating` (aucune note réelle, donc interdit). Pas de `HowTo` (plus affiché). Sur `/i18n`, réutiliser le même `SoftwareApplication`.

### 5.5 Transition depuis la landing actuelle

La home se positionne aujourd'hui (faiblement) sur la localisation par IA. Sans `/i18n`, ces signaux sont perdus. Il faut donc :
1. Publier `/i18n` avec le contenu actuel de la landing (title : `Open-source AI localization for coding agents · glot`) **le même jour** que la nouvelle home.
2. Mettre en place le lien home → `/i18n` (H2 ci-dessus) et le lien `/docs/getting-started` → `/i18n`.
3. Suivre dans la Search Console les requêtes de la home pendant 4 semaines ; si des requêtes i18n décrochent, renforcer le maillage vers `/i18n`.

---

## 6. Plan de contenu (10 pages ou articles)

| # | Page | Mot-clé principal | Contenu minimal | Liens sortants |
|---|---|---|---|---|
| 1 | `/i18n` (ex-landing) | ai localization tool, translate react app with ai | Contenu actuel + benchmark (14,1 % → 0 %) + tableau comparatif sourcé | docs, `/i18n/*`, `/compare/*` |
| 2 | `/i18n/nextjs` | next.js i18n, next-intl | `glot i18nize --routing prefix`, fichiers écrits (tiré de `frameworks.md`), pièges SEO (hreflang/sitemap avec next-intl, ce qu'on corrige ici même), exemple vérifié | `/docs/frameworks`, GitHub examples |
| 3 | `/i18n/ai-builders/lovable` | localize lovable app | Snippet de knowledge (`integrations/ai-builders-prompt.md`), bouton « Translate this app », PR en un clic | `/i18n`, `/docs/integrations` |
| 4 | `/compare/lokalise` | lokalise alternative | Tableau sourcé et daté (reprendre `docs/comparison-sources.md`), prix, quand choisir Lokalise (honnête) | `/pricing`, `/i18n` |
| 5 | `/compare/lingo-dev` | lingo.dev alternative | Idem ; différenciateur : validation déterministe + largeur en pixels | `/i18n`, `/docs/quality` |
| 6 | `/localize/ios-swiftui` | swiftui localization, xcstrings | Formats mobiles (`examples/mobile-formats/ios`), pluriels CLDR, commande | `/docs/frameworks`, glossaire CLDR |
| 7 | `/glossary/multilingual-copy-testing` | a/b testing translations | Définition, différence avec l'A/B classique et le multivarié, problème du trafic par langue, bandit, holdout | **Landing**, `/docs/experiments` |
| 8 | Blog : « Multivariate vs A/B vs bandit for multilingual copy » | multivariate testing vs a/b testing | Calcul du trafic nécessaire par langue × segment, pourquoi les petites locales demandent un bandit | **Landing**, glossaire |
| 9 | Blog : « We let an LLM translate 1,000 strings: 14.1 % broke » | translate app with ai, llm translation errors | Méthode du benchmark (`docs/bench`), catégories d'erreurs (placeholders, pluriels, débordement) | `/i18n`, `/docs/benchmark` |
| 10 | `/glossary/cldr-plurals` (+ icu-messageformat) | cldr plural rules | Tableau des catégories par langue (pl, ar, ja), exemples ICU/i18next/MF2, erreurs courantes | `/docs/concepts`, `/i18n` |

Cadence suggérée : P0/P1 techniques → pages 1 et 7 (en même temps que la nouvelle landing) → 2, 3, 4 → 5, 6, 8 → 9, 10. Chaque page porte une date de mise à jour visible et des sources (signal E-E-A-T, et ce sont ces pages que les moteurs génératifs citent).

KPIs : impressions et clics GSC par cluster (marque / i18n / experiments / compare), pages indexées vs soumises, hreflang sans erreur, CWV terrain « bon » sur 100 % des URL, clics vers GitHub et npm depuis les pages organiques.

---

## Sources (consultées le 2026-10-18)

- Hreflang et sitemap avec Next.js / next-intl : [dev.to — multilingual sitemap next-intl](https://dev.to/oikon/implementing-multilingual-sitemap-with-next-intl-in-nextjs-app-router-1354), [General Translation — multilingual Next.js SEO](https://generaltranslation.com/blog/multilingual-nextjs-seo), [eastondev — Next.js i18n SEO](https://eastondev.com/blog/en/posts/dev/20251225-nextjs-i18n-seo/), [dev.to — config-driven sitemap](https://dev.to/nxfold_9a37c0ceb3a755db04/building-a-sitemap-that-cant-go-stale-a-config-driven-architecture-for-nextjs-app-router-2djc) ; référence : Google Search Central, « Localized versions of your pages ».
- Core Web Vitals / INP 2026 : [metadigital](https://www.metadigital.co.nz/core-web-vitals-in-2026-what-they-measure-and-whats-actually-worth-fixing/), [seologist](https://www.seologist.com/knowledge-sharing/core-web-vitals-rankings-fixes/), [anim.hr](https://anim.hr/en/blog/core-web-vitals-2026).
- Fin des résultats enrichis FAQ (7 mai 2026) et HowTo : [ppc.land](https://ppc.land/google-kills-faq-rich-results-what-seos-saw-coming-since-2019/), [jsonld.com](https://jsonld.com/google-faq-rich-results-deprecated/), [elevarus](https://elevarus.com/faq-rich-results-deprecated-2026/), [dev.to — don't delete the Q&A markup](https://dev.to/jangwook_kim_e31e7291ad98/faq-rich-results-are-dead-dont-delete-the-qa-markup-560a).
- llms.txt : [Rankability tracker](https://www.rankability.com/llms-report/), [Digital Applied (août 2026)](https://www.digitalapplied.com/blog/llms-txt-in-practice-adoption-evidence-2026), [Presenc AI](https://presenc.ai/research/state-of-llms-txt-2026).
- Librairies i18n (repères de demande) : [PkgPulse](https://www.pkgpulse.com/guides/best-i18n-libraries-react-2026), [SimpleLocalize](https://simplelocalize.io/blog/posts/the-most-popular-react-localization-libraries/), [WeWeb](https://www.weweb.io/blog/localization-in-react-js-with-i18next-guide).
- Concurrence copy/personnalisation : [Fibr — A/B testing tools 2026](https://fibr.ai/ab-testing/ab-testing-tools), [SEATEXT — A/B testing translations vs multivariate](https://seatext.seatext.com/guides/a-b-testing-translations-vs-multivariate-testing-which-is-better-for-localization-6a920364), [Tofu — B2B personalization](https://www.tofuhq.com/post/best-ai-tools-for-b2b-landing-page-personalization), [Contentsquare — AI A/B testing](https://contentsquare.com/blog/ai-powered-ab-testing/).
- Concurrence i18n / AI builders : [openalternative — Lingo vs Tolgee](https://openalternative.co/compare/lingo/vs/tolgee), [Common Ninja — translate a Lovable website](https://www.commoninja.com/blog/how-to-translate-a-lovable-website), [globalize-now (Lovable i18n skill)](https://globalize-now.lovable.app/).
