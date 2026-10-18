# Positionnement — landing « expériences de copy par segment » (v1, 2026-10-18)

> Document de travail pour la nouvelle landing glotlang.com. Toutes les sources web ont été consultées le
> **2026-10-18** ; les prix des concurrents viennent souvent d'agrégateurs (marqués *tiers*) parce que la plupart ne les
> publient pas. Chiffres glot : uniquement ceux reproductibles depuis le repo (§6). Contrainte du fondateur : les mots
> « A/B testing », « A/B test » et « AB testing » n'apparaissent **nulle part** sur la landing.

---

## 0. En une page

**Proposition de valeur (une phrase)**
> **glot écrit des variantes de votre copy, prouve qu'elles ne cassent rien dans aucune langue, les sert à chaque
> audience sans flicker, et renvoie la gagnante dans votre repo.**

EN (pour `en.json`) : *“Your copy, tested in every language. AI writes the variants, code proves they can't break,
each audience gets its own winner, and it lands in Git.”*

**Catégorie** : *copy experiments multilingues* — sous-catégorie neuve entre les outils d'expérimentation (VWO,
Optimizely, GrowthBook, PostHog) et les plateformes de localisation (Lokalise, Crowdin, Tolgee, Lingo.dev). Aucun des
16 concurrents étudiés ne combine variantes générées + validation déterministe dans toutes les locales + livraison
SSR/edge par segment + retour dans Git (§2).

**Vocabulaire recommandé** (détail §4) : **« expériences de copy »** (EN *copy experiments*) comme nom de fonction ;
**« la copy qui apprend »** (EN *copy that learns*) comme promesse ; **« versions »** à la place de « variante B » ;
**« audiences »** à la place de « segments » côté marketing ; **« la version qui convertit »** à la place de
« gagnant d'un test ».

**3 piliers**
1. **Rien ne casse** — chaque version, dans chaque langue, passe le même validateur que `glot translate`
   (placeholders, pluriels CLDR, glossaire, largeur en pixels) **et** des règles éthiques déterministes (fausse
   urgence, faux compteurs, chiffres non sourcés, confirmshaming, prix/mentions légales modifiés).
2. **Chaque audience a son texte** — décision à l'edge, rendu côté serveur : pas de flash du texte officiel, pas de
   cookie par défaut, glot jamais sur le chemin critique ; statistiques bayésiennes par audience avec garde-fous
   (7 jours mini, budget d'erreur partagé, alarme SRM).
3. **Le gagnant devient du code** — « Promouvoir partout » écrit les textes gagnants dans vos catalogues, dans
   toutes les langues, avec historique (CDN ou pull request). Pas de couche JS à maintenir à vie.

**ICP prioritaire** : l'ingénieur·e produit / growth d'une SaaS seed→série B, Next.js + next-intl, déjà en 3+
langues, qui code avec un agent (Claude Code, Cursor, Codex), sans équipe CRO, 10 k–300 k visiteurs/mois (§3).

---

## 1. Ce qu'on vend réellement (état du repo au 2026-10-18)

| Élément | Ce que fait le code | Où | Prudence pour la landing |
|---|---|---|---|
| Groupes de clés × toutes locales | Une expérience = `home.hero.{title,subtitle,cta}` dans N langues ; une « intention » (preuve sociale, bénéfice direct…) par version | `web/content/docs/experiments.md` | — |
| Génération IA | LLM écrit en langue source à partir d'une intention, puis pipeline de traduction normal ; une version n'est activable que si **toutes** les (clé, locale) valident | `docs/design/segmented-copy-experiments.md` §3 | Dire « générées *ou* écrites par vous » |
| Validateur | placeholders, ICU/i18next/MF2, pluriels CLDR, glossaire, largeur mesurée | `README.md`, `quality.md` | Largeur = nécessite `glot measure` |
| Règles éthiques | `fake_urgency`, `fake_counter`, `unprovable_claim`, `superlative`, `unsourced_number`, `price_or_legal`, `confirmshaming`, `forbidden_term` ; refus de l'intention « dark pattern » | `crates/core/src/policy.rs`, `crates/llm/src/copywriting.rs` | « aucun signal rouge connu », **jamais** « conforme » (le code le dit lui-même) |
| Clés interdites | clés taguées `pricing` / `legal` refusées en expérience | `experiments.md` | — |
| Audiences | pays/région, locale, appareil, source (UTM/referrer), visiteur nouveau/récurrent (avec consentement), attributs app déclarés ; ≤ 4 audiences + `*` | `experiments.md` | « jusqu'à 4 audiences par expérience » |
| Livraison | `glotProxy` (Next proxy) / Cloudflare Worker décide localement depuis un JSON en cache ; SSR rend la version ; expositions envoyées après la réponse ; glot injoignable → texte officiel | `experiments.md` | **Sans flicker = SSR/edge.** Une SPA client-only peut flicker |
| Cookies | défaut `persistence: none` → aucun cookie, `sessionStorage` ; option cookie de session sans identifiant | `docs/legal/experiments-privacy.md` | « sans cookie par défaut », **pas** « sans consentement » : `sessionStorage` reste dans le champ d'ePrivacy 5(3) (EDPB 2/2023) |
| SEO | jamais de décision selon l'user-agent ; même URL, même canonical ; bots non comptés | `experiments.md` FAQ | Conforme à la doc Google (§3.4) |
| Stats | Thompson top-two par cellule avec prior partiel global, 500 visiteurs/bras et 7 jours mini, seuil relevé par look × bras × cellule, SRM, tranches exploratoires non décisionnelles, métrique revenu, holdout | `experiments.md`, `crates/server/src/experiments/stats*` | — |
| Promotion | « partout » → catalogues de toutes les locales (CDN ou PR) ; « pour cette audience » → personnalisation, re-test à 90 j | `experiments.md` | Statut **expérimental** de l'API à afficher (badge « Labs ») |
| Agents | MCP `experiment_create/results/promote`, CLI `glot labs experiments` | design §7 | Vérifier que le MCP est livré avant de le montrer |

---

## 2. Paysage concurrentiel

### 2.1 Outils d'expérimentation / optimisation de site

| Outil | Positionnement / promesse (citée) | Prix d'entrée | ICP | Ce qu'il ne fait pas (pour notre angle) |
|---|---|---|---|---|
| **VWO** (Wingify) | Plateforme d'expérimentation ; « AI-Based Text Recommendations: generate alternate text recommendations for the existing copy » [1] | Plans Growth/Pro/Enterprise sur démo [1] ; *tiers* : Growth ≈ 299–665 $/mois, médiane contrats ≈ 17 k$/an [2][3] | Équipes CRO mid-market | Pas de validation multilingue des variantes ; éditeur client-side (flicker cité dans des avis [4]) |
| **Optimizely** | « Test everything, regret nothing. » ; agents IA ; promet « fast, flicker-free experiences » sans chiffre [5] | Sur devis ; *tiers* : 36 k–80 k$/an pour Web Experimentation [6] | Grands comptes, équipes d'expérimentation dédiées | Pas de localisation ; aucun contrôle des variantes par langue |
| **PostHog Experiments** | Expériences incluses avec les feature flags, 1 M requêtes/mois gratuites, pas de siège [7] | Gratuit puis à l'usage | Startups dev-led | No-code web : « brief delay as PostHog loads » ; SPA « not a good fit » [8] ; pas de génération ni de validation de copy |
| **GrowthBook** | « Learn faster with predictable pricing », open source, bayésien partout [9] | Free 3 users ; Pro 40 $/siège/mois ; bandits et AI Visual Editor en Pro [9] | Data teams, warehouse-native | Pas de copy multilingue validée ; bandits contextuels en Enterprise seulement [9] |
| **Statsig** (racheté par OpenAI, annoncé le 2025-09-02, ≈ 1,1 Md$ [10]) | « The modern product development platform » [11] | Gratuit 2 M events ; Pro 150 $/mois [11] | Équipes produit/eng | Pas de copy, pas de localisation |
| **LaunchDarkly** | Feature management + Guarded Releases, Experimentation en plan Foundation [12] | Free Developer, puis par service connection + usage [12] | Eng orgs | Rollouts de code, pas de texte |
| **Mutiny** | A **pivoté** : « Deliver the buyer experience that wins enterprise deals — the sales assistant built for customer-facing work » [13] ; *tiers* : 36 k–60 k$/an [14] | Sur devis | ABM enterprise | N'est plus vraiment un outil de personnalisation de site → espace libéré |
| **Webflow Optimize** (ex-Intellimize) | Tests + personnalisation + optimisation IA autonome [15] | *tiers* : 399–1 199 $/mois [15] | Sites Webflow, marketing | Lié à Webflow ; pas de validation multilingue |
| **Coframe** | « Maximize revenue from the demand you already have » ; copy, code et images IA « personalized by segment » ; « AI protection: AI content needs your approval » [16] ; 9,3 M$ seed [17] | Non publié (*tiers* 50–799 $/mois) [18] | Growth/CRO teams | **Le plus proche** de notre angle, mais : garde-fous = approbation humaine, pas de règles déterministes ; localisation non mentionnée [16] |
| **Unbounce Smart Traffic** | Routage IA du trafic vers la meilleure page | Plan Optimize ≈ 249 $/mois, 50 k visiteurs (*tiers*) [19] | PME / landing pages paid | Pages, pas catalogues d'app ; pas de multilingue validé |
| **Vercel Flags SDK** (alternative DIY) | « no layout shift, and no flashing of the wrong experiment » via precompute [20] | Gratuit (SDK) | Devs Next.js | Infrastructure seule : ni copy, ni stats, ni langues ; API `unstable_` |

### 2.2 Localisation

| Outil | Positionnement | Prix d'entrée | Expériences de copy ? |
|---|---|---|---|
| **Weglot** | Traduction de site no-code | Free 2 000 mots ; Starter 15 €/mois → Extended 699 €/mois (*tiers*) [21] | Non |
| **Lokalise** | Plateforme pour équipes produit/design, routage IA multi-LLM | Dès 149 $/mois [22] | Non |
| **Phrase Strings** | TMS entreprise | *tiers* : ≈ 525 $/mois (Software UI/UX) [23] | Non |
| **Crowdin** | Écosystème le plus large, QA ICU par défaut [22] | Plans payants, essai 14 j [22] | Non |
| **Tolgee** | Open core, édition in-context | Free cloud, cloud dès 58 €/mois [22] | Non |
| **Lingo.dev** | « Localization engines » : localisation comme infrastructure (v1.0, mars 2026) [24] | CLI gratuite BYOK, hébergé 0 $ + usage [22] | Non |
| **SeaText** (niche) | Agent de test IA + agent de traduction, « promotes winners per locale », Webflow [25] | n.c. | Oui, mais Webflow-only, pas de validateur ICU/pluriels |

### 2.3 Lecture

- **Les outils d'expérimentation ignorent les langues.** Aucune page produit VWO/Optimizely/Coframe consultée ne
  mentionne la localisation [1][5][16]. Leur variante est un texte, pas un message ICU avec pluriels et placeholders.
- **Les outils de localisation ignorent la conversion.** Ils s'arrêtent à « traduit et relu ». Aucun ne mesure quel
  texte convertit, par langue.
- **Les garde-fous IA du marché sont humains** (approbation chez Coframe [16]) ou absents. glot est le seul à
  **refuser par du code** une variante qui casse un pluriel polonais, déborde d'un bouton allemand ou invente « plus
  que 2 places ».
- **Le prix est une arme** : VWO ≈ 300 $+/mois, Optimizely ≥ 36 k$/an, Unbounce Smart Traffic ≈ 249 $/mois, Mutiny
  ≥ 36 k$/an — contre 19 €/mois chez glot (Pro).
- **Concurrent à surveiller : Coframe** (même promesse « copy IA par segment », financé). Différenciation : multilingue
  validé, open source, Git, règles éthiques déterministes, prix public.

---

## 3. ICP, douleurs, objections

### 3.1 ICP prioritaire

**« L'ingénieur·e growth d'une SaaS internationale qui code avec un agent »**

- SaaS B2B ou prosumer, seed → série B, 5–80 personnes ; Next.js App Router + next-intl (le chemin sans flicker est
  livré pour cette stack) ou site statique derrière Cloudflare.
- Déjà en **3+ langues** (ou en train d'y passer — c'est la porte d'entrée glot existante : `glot localize`).
- 10 k–300 k visiteurs/mois : assez pour une expérience globale, pas pour 10 segments.
- Pas d'équipe CRO ; le fondateur ou un PMM écrit la copy ; un·e dev la met en ligne.
- Utilise déjà Claude Code / Cursor / Codex (glot a déjà un MCP et des règles d'agent).
- **Déclencheur** : ouverture d'un nouveau marché (DE, JP, BR…) dont la conversion est inférieure à l'anglais ;
  refonte du hero ; sortie de Google Optimize jamais remplacée.

**ICP secondaires** : (a) PMM d'une scale-up déjà multilingue qui veut tester sans ticket dev à chaque fois ;
(b) agences/studios Next.js qui livrent des sites multilingues (effet multiplicateur).
**Hors cible (le dire)** : équipes CRO enterprise sous Optimizely ; sites Webflow/Shopify no-code ; tests de prix
(interdits par conception).

### 3.2 Jobs-to-be-done

| Job | Formulation |
|---|---|
| Principal | « Quand j'ouvre un marché, je veux savoir quel message convertit *dans cette langue*, sans devenir expert en stats ni casser le site. » |
| Fonctionnel | « Générer 2–3 alternatives crédibles du hero dans 6 langues, sans relire 18 textes à la main. » |
| Risque | « Ne jamais mettre en ligne un pluriel cassé, un bouton qui déborde ou une promesse illégale, même écrite par une IA. » |
| Émotionnel | « Arrêter de débattre de la copy en réunion : laisser les visiteurs trancher. » |
| Organisationnel | « Que le gagnant ne vive pas dans un outil tiers : qu'il soit dans le repo, relu en PR, comme le reste. » |

### 3.3 Douleurs, avec verbatims

| Douleur | Verbatim (source, date) |
|---|---|
| Trafic faible | « a small startup may only be able to reliably detect a 60% effect in the conversion rate » — HN, 2014-03-31 [26] ; « most B2B landing pages get fewer than 5,000 visitors per month » (guide vendeur) [27] |
| Flicker | « Google Optimize flickers the screen with React » — HN, 2022-01-07 [28] ; « I'm just hoping someone comes up with perfect way I can just put a script in the head and not worry about flickering » — HN, 2013-03-12 [29] ; « there is a slight flicker on some tests, which can slightly skew results » — avis VWO, Capterra 2018 [4] |
| Prix | « it was $49/months a few years ago for basic use and now you can't do anything for less than $299/month » — avis VWO, Capterra [30] |
| Texte qui déborde en traduction | « layouts look fine in English, then the German translation (+35% length on average) overflows a fixed-width button in production » — HN, 2026-07-09 [31] |
| Copy IA de mauvaise qualité | « please don't have an LLM write marketing copy and then shovel the first draft onto the internet » — HN, 2025-06-18 [32] ; « Not paying my marketing team to put out obliviously AI generated texts. People will notice » — HN, 2025-04-15 [33] |
| Localisation = enjeu de revenu | 76 % des acheteurs en ligne préfèrent acheter avec des informations dans leur langue, 40 % n'achètent jamais sur un site dans une autre langue (CSA Research, 8 709 consommateurs, 29 pays, 2020-07-07) [34] |

> Reddit : l'API de recherche a refusé nos requêtes et la recherche web n'a renvoyé aucun fil Reddit exploitable
> sur ces sujets ; les verbatims Reddit restent **à collecter à la main** (r/SaaS, r/nextjs, r/webdev,
> r/localization) avant le lancement (§8, semaine −3).

### 3.4 Objections et réponses (à mettre en FAQ landing)

| Objection | Réponse factuelle | Appui |
|---|---|---|
| « Mon trafic est trop faible » | Le tableau de bord estime la durée **avant** de lancer (« ≈ 19 jours pour détecter +20 % ») ; une audience qui demanderait > 90 jours est fusionnée ; « une expérience globale vaut mieux que quatre audiences affamées ». Tester des *intentions* différentes (gros effets), pas des virgules. Aucun gagnant avant 500 visiteurs/version et 7 jours. | `experiments.md` ; HN [26] |
| « RGPD / cookies » | Aucune IP, UA, route ni identifiant stockés : seulement des compteurs agrégés (expérience, audience, version, jour). Pas de cookie par défaut. GPC/DNT → texte officiel, rien compté. glot **informe, ne conclut pas** : note DPO fournie. Ne pas écrire « exempté de consentement » : `sessionStorage` est dans le champ d'ePrivacy 5(3) ; l'exemption CNIL vise la mesure d'audience strictement nécessaire [35]. | `docs/legal/experiments-privacy.md` |
| « Flicker » | La version est choisie à l'edge et dans le HTML initial : l'hydratation est identique, pas de swap. Décision locale sur un fichier en cache, 0 appel bloquant ; glot en panne → texte officiel. Limite honnête : SPA sans SSR = chemin client, qui peut flicker. | `experiments.md` ; PostHog reconnaît le délai côté client [8] |
| « SEO / cloaking » | Google interdit de montrer un contenu différent à Googlebot et recommande canonical + tests courts [36]. glot ne décide jamais selon l'user-agent : un crawler tire une version comme tout le monde, même URL, même canonical ; bots non comptés ; versions sémantiquement équivalentes ; re-test à 90 jours. | `experiments.md` FAQ ; [36] |
| « L'IA va écrire n'importe quoi » | Trois verrous : (1) le validateur refuse placeholder perdu, pluriel incomplet, glossaire violé, texte trop large — dans **chaque** langue ; (2) les règles éthiques refusent fausse urgence, faux compteurs, « n°1 », chiffres non déclarés, prix/mentions légales modifiés, confirmshaming ; (3) les clés `pricing`/`legal` sont interdites et une version reste brouillon jusqu'à approbation si demandé. Une version n'est servie que si **toutes** ses langues passent. | `policy.rs`, `experiments.md` ; HN [32][33] |
| « Encore un script tiers » | Pas de script de test dans le navigateur pour le chemin SSR : un proxy de 3 lignes, `withExperiments` dans `i18n/request.ts`, `track("signup")`. Vos `t()` ne changent pas. Le gagnant promu devient un fichier JSON dans votre repo : on peut désinstaller glot et garder le résultat. | `experiments.md` |
| « Et si je veux des tests de prix ? » | Non, par conception. C'est un argument, pas une limite : glot teste des mots, pas des prix ni des promesses. | `experiments.md` |
| « C'est expérimental ? » | Oui (API `/xp` susceptible de changer) — l'afficher. Le moteur stats et le validateur sont testés (simulations §6). | `experiments.md` |

---

## 4. Vocabulaire : remplacer « A/B »

### 4.1 Pourquoi ne pas dire « A/B testing »

1. **C'est techniquement faux pour glot.** Le mode par défaut n'est pas un split 50/50 fixe : c'est une allocation
   adaptative (Thompson top-two) par audience, avec 3–4 versions et une promotion vers le code. Dire « A/B » sous-vend.
2. **Le terme importe les objections du segment.** « A/B testing » déclenche immédiatement trafic insuffisant, flicker,
   prix VWO/Optimizely, et « Google Optimize est mort ». Notre ICP a déjà décidé que « l'A/B testing n'est pas pour
   nous » (§3.3).
3. **Il nous place dans une catégorie où l'on perd** : face à VWO/Optimizely/GrowthBook sur leurs critères (éditeur
   visuel, intégrations analytics, années de stats). Dunford : choisir le cadre de référence où nos attributs uniques
   comptent — ici, *la copy dans toutes les langues*.
4. **L'objet testé est la copy, pas la page.** Personne d'autre ne nomme ce marché ; on peut le posséder.

### 4.2 Lexique recommandé

| À éviter | Landing EN | Landing FR | Pourquoi |
|---|---|---|---|
| A/B testing (catégorie) | **copy experiments** | **expériences de copy** | Déjà le nom dans le produit et les docs ; concret, dev-friendly |
| (promesse) | **copy that learns** | **la copy qui apprend** | Dit l'allocation adaptative sans jargon ; mémorable |
| variante B / bras | **version** (une *intention* : « social proof », « direct benefit ») | **version** / **angle** | Un humain parle de versions et d'angles, pas de bras |
| segment | **audience** (« German visitors from search ») | **audience** | Langage marketing ; *segment* reste dans la doc/API |
| test gagnant / significatif | **the version that converts** · **a clear winner** | **la version qui convertit** · **un gagnant net** | Résultat, pas méthode |
| split / allocation | **traffic shifts to what works** | **le trafic va vers ce qui marche** | Dit le bandit sans le mot |
| significance / p-value | **confidence** + fourchette (« +12 %, probably +4 to +20 %, 97 % confidence ») | **confiance** + fourchette | Format déjà utilisé dans le produit |
| personnalisation | **each audience keeps its winner** | **chaque audience garde son gagnant** | Distingue « promouvoir partout » / « pour cette audience » |
| déployer le gagnant | **ship the winner to Git** | **le gagnant part dans Git** | Pilier 3 |

Verbes : *try*, *compare*, *run an experiment* (autorisé : ce n'est pas « A/B »). Par cohérence, bannir aussi
*split test*, *multivariate* et *CRO* de toute la landing, FAQ comprise : une règle simple tient mieux.

### 4.3 Conséquences hors landing

- `web/content/docs/experiments.md` parle de **« Fixed A/B »** et la conception d'**« A/B fixe »** dans l'étape 4 du
  dashboard : renommer en **« Fixed split »** / **« Répartition fixe »** pour que la landing ne renvoie pas vers un
  écran qui contredit la règle. (Pas fait ici : aucun changement de code demandé.)
- Le trafic de recherche existe sur « A/B testing » : décider si les **docs** et une page `/compare` gardent le terme
  pour le SEO (« glot n'est pas un outil d'A/B testing : … »). Recommandation : oui dans les docs, non sur la landing.

---

## 5. Positionnement (April Dunford)

| Composant | Contenu |
|---|---|
| **Alternatives compétitives** (ce que l'ICP fait aujourd'hui) | (1) **Rien** : la copy est décidée en réunion, traduite une fois, jamais testée — de loin la plus fréquente ; (2) un test en anglais seulement via PostHog/GrowthBook, puis traduction du gagnant (« le gagnant anglais marche partout ») ; (3) VWO/Optimizely/Unbounce trop chers pour leur trafic ; (4) DIY : Vercel Flags + feature flags + tableur ; (5) Coframe / Webflow Optimize pour les équipes marketing. |
| **Attributs uniques** | Validateur déterministe sur chaque version × chaque locale (placeholders, pluriels CLDR, glossaire, largeur en pixels) ; règles éthiques déterministes ; décision edge + SSR (pas de flash), sans cookie par défaut, glot hors chemin critique ; stats bayésiennes par audience avec contrôle des faux gagnants ; promotion vers les catalogues/Git ; open source MIT, BYOK, self-host ; pilotable par un agent (MCP). |
| **Valeur** (ce que ça permet) | Savoir quel message convertit **dans chaque marché** sans casser l'app ni risquer un claim illégal ; passer de « une copy pour tous » à « la bonne copy par audience » sans équipe CRO ni contrat à 5 chiffres ; garder le résultat dans le code. |
| **Clients qui s'en soucient le plus** | ICP §3.1 : SaaS Next.js multilingue, dev-led, avec agent de code, sans équipe CRO. |
| **Catégorie** | *Copy experiments multilingues* — présentée comme **l'étape d'après la localisation** : « vous avez traduit ; maintenant, apprenez ce qui convertit dans chaque langue ». Permet de réutiliser la crédibilité glot (benchmark 0 % cassé) plutôt que d'arriver en nouvel entrant sur l'expérimentation. |
| **Tendance pertinente** | (a) les agents de code écrivent déjà la copy → besoin de garde-fous par code ; (b) Digital Fairness Act annoncé (proposition attendue fin 2026) contre fausse urgence et dark patterns [37] ; (c) fin de Google Optimize (2023) jamais remplacée chez les petites équipes. |

### 5.1 Messages par pilier (hiérarchie de landing)

**Hero**
- EN : **“Copy that learns, in every language.”**
  Sub : *“glot writes versions of your headline, proves they can't break in any locale, serves each audience its own
  winner with no flicker, and ships the winner to Git.”*
- Alternatives : *“Find the words that convert in every market.”* · *“Your hero, tested in 8 languages. Nothing
  breaks.”*
- FR : **« La copy qui apprend, dans chaque langue. »**

**Pilier 1 — “Nothing ships broken.”** Cartes de refus réelles (format existant de la landing) : pluriel polonais
manquant, bouton allemand 276 px pour 120 px, et **nouvelles** cartes éthiques : « *Plus que 2 places* : fake
urgency — not backed by the official text or a declared fact », « *N°1 en Europe* : unprovable claim ».
**Pilier 2 — “Every audience gets its own words.”** Schéma navigateur → edge → SSR ; puces : first HTML, no cookie by
default, glot down = official text ; carte résultat : « Germany · search — *Direct benefit* : +12 % signups (probably
+4 to +20 %), 97 % confidence. ~3 days left. »
**Pilier 3 — “The winner becomes code.”** Capture d'une PR « promote: home.hero — Direct benefit (en, fr, de, ja) »
avec diff JSON.

---

## 6. Preuves disponibles (et comment les citer)

| Preuve | Chiffre exact | Source reproductible | Formulation landing |
|---|---|---|---|
| Benchmark traduction | **0,0 % cassé** avec glot vs **14,1 %** (Haiku) / **8,8 %** (Sonnet) en prompt brut ; 320 messages ICU par modèle et condition, 8 locales ; refus 1,9 % / 3,1 % ; juge indépendant FormatJS + `Intl.PluralRules` d'accord | `bench/results/2026-10-18-real`, `web/content/docs/benchmark.md` | « 0 % broken strings in our public benchmark, vs 14.1 % without glot » + lien rapport. Ne pas dire « 0 % d'erreurs » (le benchmark ne mesure pas le sens) |
| Faux gagnants (A/A segmenté) | **0/200** selon le fondateur ; le test exige ≤ 5 % (≤ 10/200) sur 3 audiences × 3 versions identiques, analyses quotidiennes 6 semaines ; alarmes SRM ≤ 3/200 | `crates/server/src/experiments/stats/tests.rs` `simulation_aa_false_positives` (`cargo test --release -p glot-server --lib simulation -- --nocapture`) | « 200 simulated experiments where every version was identical: 0 false winners. » — **à confirmer par la sortie du test avant publication** (voir note ci-dessous) |
| Bons gagnants par audience | **80/80** selon le fondateur = 40 simulations × 2 audiences ayant chacune un vrai gagnant différent ; le test exige ≥ 80 %, ≤ 5 % de mauvais, 0 décision avant 7 jours, et 40/40 recommandations d'audience correctes | même fichier, test « per segment » | « 2 audiences with different winners, 40 simulations: the right winner in each, 80 out of 80, never before day 7. » |
| Autres simulations | SRM détecté (10 % d'un bras perdu) ; métrique revenu (+30 % panier, même taux) trouvée ≥ 75 % ; A/A revenu ≤ 5 % ; pas de décision prématurée à faible trafic ; A/A historique ≈ 2 % faux positifs, bon bras 100/100 | même fichier ; `ARCHITECTURE.md` l. 292 | Section « Statistics you can audit » avec lien vers les tests |
| Règles éthiques | 8 règles, déterministes, refus de l'intention « dark pattern » | `crates/core/src/policy.rs` | Lister les règles ; mention « no known red flag, never “compliant” » |
| Open source | MIT, repo public | `LICENSE` | Badge existant |

> **Note de vérification (2026-10-18)** : 0/200 et 80/80 n'ont **pas** pu être reconfirmés ici — la cible de test
> `glot-server --lib` ne compilait pas au moment de la rédaction (4 erreurs sans lien avec les stats : `LinkRow.branch`
> privé, crate `toml` introuvable…), et aucun rapport du repo ne consigne ces chiffres. Les assertions du code
> garantissent seulement les seuils (≤ 10/200, ≥ 64/80) ; publier le chiffre
> **observé** avec la graine fixée (`seed_from_u64`) et le lien vers le test, jamais un chiffre arrondi.
> Toujours qualifier « simulated » : ce sont des mondes simulés, pas des clients.

**Preuves manquantes à produire avant lancement** : (1) un **dogfooding** sur glotlang.com lui-même (hero en
expérience EN/FR/DE…, résultats publiés quel qu'en soit le sens) — c'est la preuve la plus forte et la moins chère ;
(2) 2–3 design partners (agences Next.js) avec citations ; (3) une vidéo de 30 s : agent → `experiment_create` →
refus d'une version « fake urgency » en allemand → PR de promotion.

---

## 7. Pricing & packaging

### 7.1 Existant

Free 0 € / Pro 19 € / Team 79 € par mois ; quotas expériences (contrat §4.4) : **Free 1 expérience live, 0
audience** ; **Pro 5 expériences, 2 audiences** ; **Team illimité, 4 audiences** ; Insights events 10 k / 250 k /
2 M ; self-host sans limite (`docs/billing.md`).

### 7.2 Recommandation

1. **Garder les 3 plans et les prix pour le lancement.** 19 € est 10–15× sous VWO Growth / Unbounce Smart Traffic
   (≈ 249–300 $/mois, §2) et sous le seuil « carte bancaire perso » du dev ICP. Le prix bas est un message
   (« pour les équipes sans budget CRO »), pas une faiblesse — à condition de l'assumer en copy.
2. **La métrique de valeur = audiences**, pas sièges. Le packaging actuel est le bon : Free = 1 expérience globale
   (moment « aha » : voir une version gagner dans toutes les langues), Pro = audiences, Team = 4 audiences + volume.
   Libeller les plans par le bénéfice : *Free — one experiment, every language* ; *Pro — up to 2 audiences per
   experiment* ; *Team — 4 audiences, holdout, revenue goals*.
3. **Vérifier le goulot Free** : 10 k Insights events/mois ≈ quelques milliers de visiteurs exposés + conversions ;
   une expérience gratuite risque de ne jamais conclure. Soit afficher l'estimation de durée sur le plan Free, soit
   monter Free à 25 k events **pour les expositions d'expérience uniquement**. Décision fondateur.
4. **Ne pas facturer la génération IA** (BYOK reste gratuit et illimité) : cohérent avec l'ADN glot et neutralise
   l'objection coût.
5. **Revoir à 50 clients payants** : si Team > 30 % des revenus, tester Team à 99 € ; si les agences arrivent,
   envisager un plan *Agency* (projets multiples) — pas avant (YAGNI).
6. **Self-host** : le dire en grand (« Self-host it free, no limits ») — c'est l'argument RGPD le plus fort pour le
   segment EU.

---

## 8. Acquisition & plan de lancement

### 8.1 Canaux (par ordre de priorité)

| Canal | Pourquoi | Angle |
|---|---|---|
| **Hacker News (Show HN)** | ICP dev ; glot a déjà un brouillon Show HN (`docs/launch/show-hn.md`) | « Show HN: glot – copy experiments that can't break your translations » ; montrer les stats auditables et les refus éthiques |
| **Écosystème agents / MCP** | glot est déjà listé (glama.json, smithery.yaml) ; personne n'offre « crée une expérience sur le hero pour le trafic Google DE » à un agent | Démo Claude Code / Cursor ; listings MCP mis à jour avec les outils `experiment_*` |
| **Écosystème Next.js / next-intl** | Le chemin sans flicker est livré pour cette stack | Tutoriel « Zero-flicker copy experiments with next-intl in 3 lines » ; exemple `examples/experiments-demo` en template déployable |
| **Contenu SEO** (docs/blog, pas landing) | Requêtes à intention : « Google Optimize alternative », « A/B test Next.js without flicker », « German text overflow » | Articles techniques avec chiffres reproductibles ; le terme « A/B » autorisé ici (§4.3) |
| **Reddit** (r/nextjs, r/SaaS, r/webdev, r/localization) | Brouillon existant `docs/launch/reddit.md` | Post « résultats du dogfooding » plutôt que pitch |
| **Product Hunt** | Brouillon existant `docs/launch/product-hunt.md` | Après HN, avec la vidéo 30 s |
| **Agences Next.js** | Multiplicateur, 2–3 design partners | Offre Team gratuite 3 mois contre étude de cas |

### 8.2 Plan (6 semaines)

| Semaine | Actions |
|---|---|
| −4 | Dogfooding : hero glotlang.com en expérience (3 versions × 8 langues, 2 audiences). Renommer « Fixed A/B » dans docs/dashboard (§4.3). |
| −3 | Collecte manuelle de verbatims Reddit/G2 ; 10 entretiens ICP (script JTBD §3.2) ; recrutement de 2–3 agences. |
| −2 | Nouvelle landing (structure §5.1, FAQ objections §3.4) ; vidéo 30 s ; publier la sortie des simulations. |
| −1 | Tutoriel next-intl + template ; mise à jour listings MCP ; préparer Show HN / Reddit / PH. |
| 0 | Show HN (mardi–mercredi, 15 h–16 h CET) ; réponses en continu ; Reddit J+1 ; PH J+7. |
| +1/+2 | Publier les résultats du dogfooding (même si la version officielle gagne) ; étude de cas agence ; premier article SEO. |

**Métriques de lancement** : installations `glotProxy` (expériences créées) ; expériences démarrées (`start` sans
`not_ready`) ; % qui atteignent une décision ; conversion Free → Pro ; % de versions générées refusées par le
validateur/les règles (à publier : c'est la preuve du pilier 1).

---

## 9. Risques de message

- **Sur-promesse « sans flicker »** : vrai pour SSR/edge uniquement → toujours « with server rendering » en petit.
- **« Sans cookie » ≠ « sans consentement »** (§3.4) : ne jamais écrire « GDPR-compliant » ni « no consent needed ».
- **« Conforme » pour les règles éthiques** : interdit ; le code dit « no known red flag ».
- **Chiffres de simulation présentés comme des résultats clients** : toujours « simulated ».
- **Statut expérimental** : badge Labs visible ; sinon la première issue GitHub le dira à notre place.
- **Coframe** pourrait ajouter le multilingue : l'avance défendable est le validateur + l'open source + Git, pas
  « l'IA écrit des variantes ».

---

## Sources (consultées le 2026-10-18)

1. Wingify/VWO, Pricing & Plans — https://wingify.com/pricing/
2. Toolradar, VWO pricing (*tiers*) — https://toolradar.com/tools/vwo/pricing
3. CostBench, VWO plans (*tiers*, médiane 16 660 $/an sur 93 achats) — https://costbench.com/software/ab-testing/vwo/
4. Capterra, avis VWO (flicker) — https://www.capterra.com/p/147639/Visual-Website-Optimizer/reviews?page=3 ; Endorsal — https://endorsal.io/reviews/vwo/5
5. Optimizely, Web Experimentation — https://www.optimizely.com/products/web-experimentation/
6. ConversionWax, Optimizely Web Experimentation pricing (*tiers*, données Vendr) — https://www.conversionwax.com/optimizely-web-experimentation-pricing/ ; Stellar — https://gostellar.app/blog/how-much-does-optimizely-cost
7. PostHog, Experiments pricing — https://posthog.com/experiments/pricing
8. PostHog, No-code web experiments — https://posthog.com/docs/experiments/no-code-web-experiments
9. GrowthBook, Pricing — https://www.growthbook.io/pricing
10. Verdict, OpenAI acquires Statsig (2025-09) — https://www.verdict.co.uk/openai-acquire-statsig/ ; etcentric — https://www.etcentric.org/openai-acquires-product-testing-firm-statsig-for-1-1-billion/
11. Statsig, Pricing — https://www.statsig.com/pricing
12. LaunchDarkly, plans docs — https://launchdarkly.com/docs/home/account/plans ; archive sept. 2025 — https://launchdarkly.com/pricing/archive-sept2025/
13. Mutiny, homepage — https://www.mutinyhq.com/
14. Abmatic, Mutiny pricing (*tiers*, concurrent) — https://blog.abmatic.ai/mutiny-pricing
15. Digidop, Optimize by Webflow (*tiers*) — https://www.digidop.com/blog/optimize-by-webflow-the-ai-powered-cro-tool-for-marketers ; Toolradar — https://toolradar.com/tools/intellimize
16. Coframe, homepage — https://www.coframe.com/ ; docs — https://docs.coframe.com/introduction
17. Coframe seed 9 M$ (Reuters via AOL) — https://www.aol.com/news/coframe-raises-9-million-websites-130406960.html
18. AISOTools, Coframe pricing (*tiers*) — https://aisotools.com/pricing/coframe
19. CostBench / Leadpages, Unbounce pricing (*tiers*) — https://costbench.com/software/ab-testing/unbounce/ ; https://leadpages.com/blog/unbounce-pricing
20. Vercel, Flags SDK 3.2 / precompute — https://vercel.com/changelog/flags-sdk-3-2 ; https://vercel.com/blog/vercel-flags-platform-native-feature-flags
21. Capterra, Weglot pricing (*tiers*) — https://www.capterra.com/p/200736/Weglot/pricing/
22. glot, `docs/comparison-sources.md` (Lokalise, Crowdin, Tolgee, Lingo.dev, sources primaires vérifiées le 2026-10-18)
23. Toolradar, Phrase pricing (*tiers*) — https://toolradar.com/tools/phrase/pricing
24. Lingo.dev, Introducing v1 — https://lingo.dev/en/blog/introducing-lingodotdev-v1 ; TechCrunch 2025-02-18 — https://techcrunch.com/2025/02/18/lingo-dev-is-an-app-localization-engine-for-developers
25. SeaText, A/B testing translated content on Webflow — https://seatext.seatext.com/guides/what-tools-support-a-b-testing-for-translated-content-on-webflow-6a8501df
26. Hacker News, commentaire 2014-03-31 — https://news.ycombinator.com/item?id=7504814
27. Landing Doctor / Tailor, seuils de trafic (*vendeur*) — https://landingdoctors.com/guides/when-not-to-ab-test-your-landing-page ; https://tailorhq.ai/guides/traffic-thresholds
28. Hacker News, 2022-01-07 — https://news.ycombinator.com/item?id=29838924
29. Hacker News, 2013-03-12 — https://news.ycombinator.com/item?id=5364080
30. Capterra, VWO pricing reviews — https://www.capterra.com/p/147639/Visual-Website-Optimizer/pricing
31. Hacker News, Show HN 2026-07-09 — https://news.ycombinator.com/item?id=48851198
32. Hacker News, 2025-06-18 — https://news.ycombinator.com/item?id=44306613
33. Hacker News, 2025-04-15 — https://news.ycombinator.com/item?id=43694331
34. CSA Research, « Consumers Prefer their Own Language » (2020-07-07) — https://csa-research.com/Blogs-Events/CSA-in-the-Media/Press-Releases/Consumers-Prefer-their-Own-Language
35. CNIL, Cookies : solutions pour les outils de mesure d'audience — https://www.cnil.fr/fr/solutions-pour-la-mesure-daudience
36. Google Search Central, Minimize A/B testing impact in Google Search — https://developers.google.com/search/docs/crawling-indexing/website-testing
37. Lewis Silkin, Digital Fairness Act consultation (2025-07-18) — https://www.lewissilkin.com/insights/2025/07/18/european-commission-consults-on-digital-fairness-act-102ku4t ; Slaughter and May — https://thelens.slaughterandmay.com/post/102m222/digital-fairness-act-european-commission-publishes-responses-to-consultation
