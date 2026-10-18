# Landing copy — v1 (2026-10-18)

> Nouvelle landing centrée sur les **variantes de texte écrites par l'IA, vérifiées par du code, servies par segment**.
> Raisonnement en français, **copy en anglais** (langue source du site ; les autres locales sont générées par glot).
> Copy prête à coller = blocs `>` et tableaux « Copy ». Rien ici ne modifie le code.

**Contrainte stricte.** La page n'emploie jamais le nom usuel de la méthode (lettres A et B séparées par une barre oblique, avec ou sans « test »), ni « split test ». Vocabulaire de remplacement : §8. Vérification avant mise en ligne : `grep -niE "a/?b[ -]?test|split[ -]test|\bA/B\b" web/messages/en.json` doit renvoyer 0 ligne. ⚠ Deux sources existantes violent la règle et apparaîtraient si on les lie depuis la landing : `web/content/docs/experiments.md` (« **Fixed A/B** keeps the split… », « a textbook test ») et le libellé du choix d'allocation dans le dashboard (conception §6 : « A/B fixe »). À renommer en **Fixed split** (voir §8).

---

## 0. Ce que la recherche impose (sources datées)

| Constat | Conséquence pour la copy | Source (consultée le 2026-10-18) |
|---|---|---|
| Les pages écrites à un niveau de lecture 5e–7e année (US) convertissent presque 2× mieux que celles au niveau « professionnel » ; les mots difficiles sont le facteur le plus corrélé à la baisse de conversion (57 M conversions analysées). | Phrases ≤ 20 mots, verbes concrets, jargon statistique relégué aux détails (« probability », pas « posterior »). | Unbounce, *Conversion Benchmark Report 2024*, via [dmn.ca](https://dmn.ca/unbounces-2024-conversion-benchmark-report-proves-that-attention-spans-are-declining-and-so-are-conversion-rates/) et [hartzer.com](https://www.hartzer.com/blog/unbounce-2024-conversion-benchmark-report/) (rapport primaire non consulté) |
| Les internautes balaient en F / « layer cake » : titres et débuts de ligne portent le sens. | Chaque H2 doit dire le bénéfice seul ; premier mot de chaque puce = le mot-clé. | NN/g, [F-shaped pattern](https://www.nngroup.com/articles/f-shaped-pattern-reading-web-content-discovered/) (étude confirmée 11 ans après) |
| Hero : « test des 3 secondes » — quoi, pour qui, pourquoi maintenant ; outcome d'abord, mécanisme dans le sous-titre ; CTA verbe + valeur. | H1 = résultat, sous-titre = mécanisme, preuve juste sous les CTA. | Consensus praticien, faible niveau de preuve : [leadmagic.io](https://leadmagic.io/gtm-skills/landing-page-copy), [insertchat.com](https://insertchat.com/free-tools/hero-section-copy-generator) |
| Landings devtools 2026 (relevé des H1/H2 le 2026-10-18) : H1 de 2 à 8 mots, catégorie ou promesse nette — Resend « Email for developers », Supabase « Build in a weekend / Scale to millions », Cursor « Cursor is your coding agent for building ambitious software. », Linear « The product development system for teams and agents », PostHog « Your product's context layer » (avec une H2 « Shameless CTA »), Vercel « Agentic Infrastructure », Lingo.dev « The localization engineering platform ». | Court, sans adjectif creux ; ton direct, auto-dérision permise (PostHog) ; H2 en phrases de bénéfice (Lingo.dev : « Measure translation quality in languages you can't read »). | Pages d'accueil des 7 sites, H1/H2 extraits par `curl` le 2026-10-18 |
| Google : servir à Googlebot une version différente = cloaking ; garder la même URL, durée limitée, variantes proches. | Argument SEO honnête dans la FAQ (glot ne décide jamais selon l'user-agent). | Google Search Central, [Minimize testing impact in Google Search](https://developers.google.com/search/docs/crawling-indexing/website-testing) |
| Concurrent direct « IA qui réécrit la copy » : Coframe annonce de l'optimisation continue « sans attendre la significativité », par script tag ; ses « customer reports » sont des audits suggérés, pas des gains mesurés. | Notre angle différenciant : **garanties déterministes** (rien de cassé, rien de mensonger, rien de trop large) + **statistiques qui attendent** (7 jours, 500 visiteurs/bras) + **le gagnant revient dans le repo**. Ne pas citer Coframe sur la page. | [docs.coframe.com](https://docs.coframe.com/business-intelligence), [coframe.com/post/ad-to-landing-page-optimization](https://coframe.com/post/ad-to-landing-page-optimization) |
| Frameworks retenus : **PAS** pour la section problème, **BAB** pour la démo, **4U** (utile, urgent, unique, ultra-spécifique) pour noter les heroes ; StoryBrand seulement pour l'ordre global (le dev est le héros, glot le guide, le plan = 3 étapes). | Voir la justification de chaque hero. | Frameworks classiques ; pas de source chiffrée |

Contrôle d'accessibilité du texte : libellés de liens/CTA compréhensibles hors contexte (WCAG 2.2 SC 2.4.4 / 2.5.3 *Label in Name*), pas d'information portée par la seule couleur dans les résultats (SC 1.4.1), niveau de lecture visé (SC 3.1.5, AAA, indicatif). [w3.org/TR/WCAG22](https://www.w3.org/TR/WCAG22/).

---

## 1. Faits autorisés sur la page (et seulement ceux-là)

Tout chiffre de la page vient de ce tableau. Rien d'autre : pas de « teams », pas de logos clients, pas de témoignage, pas de « +X % de conversion » présenté comme mesuré.

| # | Fait | Source dans le repo |
|---|---|---|
| F1 | 320 messages ICU par modèle et condition, 8 locales (fr, de, es, pl, ru, ar, ja, zh), run du 2026-10-18 | `web/content/docs/benchmark.md` (run `bench/results/2026-10-18-real`) |
| F2 | Cassés écrits au catalogue : **0.0 %** avec glot vs **14.1 %** prompt brut (claude-haiku-5-5) ; **0.0 %** vs **8.8 %** (claude-sonnet-5-5) | idem, tableau *Summary* |
| F3 | Prompt brut, Haiku : **42.5 %** cassés en polonais, **40.0 %** en arabe | idem, *Per locale* |
| F4 | Refusés et laissés non traduits plutôt qu'écrits cassés : 1.9 % (Haiku), 3.1 % (Sonnet) | idem |
| F5 | Juge indépendant (FormatJS + `Intl.PluralRules`) : 0.0 % avec glot | idem |
| F6 | 850 chaînes = **$0.035** avec le modèle par défaut, dans notre run | `web/content/docs/faq.md` |
| F7 | Un gagnant n'est déclaré qu'après **7 jours** minimum et **500 visiteurs par bras** (défaut) | `web/content/docs/experiments.md`, `crates/server/src/experiments/stats/tests.rs` |
| F8 | Jusqu'à **4 segments** par expérience + la cellule « tous les autres » ; au moins **10 %** du trafic par bras en exploration | `experiments.md`, contrat `MAX_CELLS = 4` |
| F9 | Simulations dans la suite de tests : 200 runs sans effet réel (3 cellules × 3 bras, 6 semaines) → la CI échoue si plus de **5 %** déclarent un gagnant ; 40 runs avec effet par segment → au moins **80 %** de bons gagnants exigés, aucun avant le jour 7 | `crates/server/src/experiments/stats/tests.rs` (`simulation_aa_false_positives`, `simulation_right_winner_per_segment`). ⚠ Ce sont des **seuils assertés**, pas des taux mesurés publiés : formuler « our test suite fails if… », jamais « 97 % accurate ». |
| F10 | Config servie en cache (rafraîchie chaque minute), fallback sur le texte officiel si glot est injoignable ou config > 24 h | `experiments.md` |
| F11 | 8 règles de politique : fake urgency, fake activity counters, unprovable claims, unsourced numbers, superlatives (warning), changed prices / dropped legal mentions, confirmshaming, forbidden terms | `crates/core/src/policy.rs` |
| F12 | 8 intentions de variantes : social proof, direct benefit, honest urgency, simplicity, curiosity, authority, reassurance, segment | `crates/llm/src/copywriting.rs` |
| F13 | Plans cloud : Free = 1 expérience, sans segment ; Pro = 5 expériences, 2 segments ; Team = illimité, 4 segments ; self-hosted = tout illimité | `crates/server/src/billing.rs` (`default_plans`) — **à revérifier** si les plans Stripe changent |
| F14 | Beacon Insights < 2 kB ; overlay dev ≈ 6 kB gzip | `insights.md`, `en.json` landing.overlay |
| F15 | Pas de cookie par défaut (`persistence: "none"`, bras gardé en `sessionStorage`) ; GPC / DNT → texte officiel, rien compté | `experiments.md` FAQ |
| F16 | Définitions d'expériences versionnées dans `glot.experiments.toml` ; `glot experiments validate` tourne hors ligne en CI | `crates/cli/src/experiments.rs` |

**Exemple illustratif, pas un résultat.** « +18% in DACH, 97% probability » est la *forme* d'un résultat glot (cf. `experiments.md` : « +12 % signups (probably between +4 % and +20 %), confidence 97 % »). Sur la page, il apparaît **uniquement** dans une capture de dashboard étiquetée `Example` (voir §3.2), jamais dans un titre ni une phrase affirmative.

**« 30 languages ».** Aucun fichier du repo ne prouve 30 langues (benchmark : 8 ; site : 6 ; glot traduit vers toute locale que le modèle couvre). Copy par défaut : « in every language you ship ». Passer à « 30 languages » seulement après un run de `glot translate` + `glot check` sur 30 locales, archivé dans `bench/results/`.

---

## 2. Trois heroes

Notation 4U de 1 à 4 : Utile / Urgent / Unique / Ultra-spécifique. Les trois partagent la même rangée de preuve et les mêmes CTA ; ils sont conçus pour tourner comme bras d'une expérience glot sur `landing.hero.*` (dogfooding — note : `landing.hero.*` est tagué `marketing` dans `[qa.risk]`, donc revue humaine obligatoire, ce qui est voulu).

### Hero A — outcome (recommandé par défaut)

> **H1:** Copy that converts, in every language.
>
> **Subtitle:** Your AI writes variants of your headlines and buttons. glot checks every one in every locale, shows each country, device and traffic source the version that converts best, and sends the winner back to your repo as a pull request.
>
> **CTA 1 (dev):** `npx glot-i18n init` — button label: **Copy the command**
> **CTA 2 (SaaS):** **Start free** — hint under it: *1 experiment free. No credit card.*

- H1 : 6 mots. 4U = 4/2/3/3.
- Pourquoi : le test des 3 s passe (quoi : de la copy qui convertit ; pour qui : apps multilingues ; comment : sous-titre). « in every language » relie la nouvelle promesse au socle i18n sans le nommer.
- Risque : « converts » est une promesse de résultat ; elle est tenue par la mécanique (on garde le meilleur bras, sinon l'officiel reste), pas par un chiffre. Ne jamais ajouter « more » sans donnée.
- ⚠ « No credit card » : à vérifier dans le flux Stripe du plan Free avant publication ; sinon supprimer le hint.

### Hero B — mécanisme (le plus « glot »)

> **H1:** AI writes the variants. Code keeps them safe.
>
> **Subtitle:** Generate new versions of any headline in every language. Deterministic checks refuse broken placeholders, missing plurals, glossary misses, text too wide for its button and fake urgency, before a single visitor sees them. Each audience gets the version that wins.
>
> **CTA 1:** `npx glot-i18n init` · **Copy the command**
> **CTA 2:** **Start free**

- H1 : 8 mots. 4U = 3/1/4/3.
- Pourquoi : prolonge l'actuel « Your agent localizes. Code guarantees. » (cohérence de marque, et c'est l'objection n° 1 des devs envers la copy IA : « elle va casser / mentir »). PAS compressé : le problème est implicite dans « keeps them safe ».
- Risque : moins clair sur le gain business pour un PM ou un growth marketer. À privilégier pour le trafic `source = social` (HN, Reddit, X).

### Hero C — segment (le plus spécifique)

> **H1:** Every market gets its best headline.
>
> **Subtitle:** German search traffic and French mobile visitors don't respond to the same words. glot lets your AI write variants, checks them in every locale, serves each segment its winner from the first byte, with no flicker and no cookie, and tells you in plain words what won and how sure it is.
>
> **CTA 1:** `npx glot-i18n init` · **Copy the command**
> **CTA 2:** **Start free**

- H1 : 6 mots. 4U = 4/2/4/4.
- Pourquoi : c'est la seule promesse qu'un outil de traduction ou un outil d'optimisation mono-langue ne peut pas faire seul. L'exemple concret (Allemagne/search, France/mobile) rend « segment » tangible sans jargon.
- Risque : suppose déjà du trafic multi-pays. Idéal comme bras pour `source = search` et pour les visiteurs hors US.
- Note : segments = plans Pro et Team (F13). Le CTA « Start free » reste honnête (le Free donne 1 expérience, sans segment) ; ne pas écrire « segment for free ».

### Éléments communs du hero

| Élément | Copy |
|---|---|
| Badge au-dessus du H1 | Open source · MIT · Bring your own LLM key |
| Command hint (sous CTA 1) | Detects your i18n stack and sets up glot. Experiments run on glot cloud or your own server. |
| Rangée de preuve (sous les CTA) | `0.0% broken strings vs 14.1% without glot¹` · `MIT licensed` · `★ {count} on GitHub` · `npm v{version}` |
| Note ¹ (lien vers /docs/benchmark) | 320 ICU messages, 8 locales, claude-haiku-5-5, October 18, 2026. Same prompts, same model. |
| Visuel (alt text) | Dashboard: three AI-written variants of a German hero, one refused by glot for fake urgency, the other two serving in Germany, Austria and Switzerland with their probability of being best. |
| Légende du visuel | Example data. Every refusal shown is the real message glot prints. |

Pourquoi le benchmark de traduction comme preuve d'un produit de variantes : les variantes passent par **le même validateur** (conception §3). Le dire explicitement dans la section 3.3.

---

## 3. Page complète, section par section

Ordre (StoryBrand allégé) : hero → problème → démo → 3 étapes → garanties → résultats → pour qui → intégrations → socle i18n → FAQ → pricing → CTA final. Longueur cible : 1 100–1 400 mots de copy visible (page devtool longue, mais chaque section se lit en 10 s par ses titres).

### 3.1 Problème (PAS)

> **Eyebrow:** The problem
> **H2:** Your best headline in English is a guess everywhere else.
>
> - **One text for everyone.** The headline you wrote for US visitors on desktop is the one German search traffic and Japanese mobile users see too.
> - **AI can write ten versions in a second.** It can also drop a `{price}` placeholder, forget the Polish plural forms, overflow a 112 px button or invent "only 3 left today".
> - **Testing copy by hand doesn't scale.** One variant, one language, a script that flickers on load, a dashboard full of percentages nobody trusts, and a winner someone has to copy back into the code.
>
> **Kicker:** glot makes the AI part safe and the testing part automatic.

- Agitation chiffrée possible (F3) en note sous la deuxième puce : *Asked to "translate this JSON", claude-haiku-5-5 broke 42.5% of Polish messages in our benchmark.* — lien /docs/benchmark.

### 3.2 Mécanisme / démo (BAB)

> **Eyebrow:** See it work
> **H2:** From one headline to the right one for each market.
>
> **Before** · `home.hero.title` · en — *Ship your invoices in minutes.* Same text for every visitor, in every country.
>
> **After** (three panels, read left to right)
> 1. **Variants written.** "Direct benefit", "Social proof", "Simplicity": three versions, in English, German and French.
> 2. **One refused.** `fake_urgency: "Only today" is not backed by your official text or a declared fact.` The other two pass placeholders, plurals, glossary and pixel width in every locale.
> 3. **Winners served.** Germany, Austria, Switzerland: "Simplicity" · **+18% signups · 97% probability it's the best** · Everyone else: your official text stays.
>
> **Caption:** Example data. Refusal messages are glot's, word for word.
>
> **Bridge:** One click on **Promote** and the winning texts land in your catalogs, in every locale, through a pull request.

- C'est **le seul** endroit où apparaît « +18% … 97% », dans une maquette étiquetée `Example`. Le texte refusé doit être le message réel de `policy.rs` au moment de la capture (le libellé ci-dessus est une approximation à remplacer).
- Option interactive (reprise du playground WASM existant « Can you break glot? ») : « Write a variant. Try to sneak in fake urgency. » si le WASM expose `policy::check` ; sinon garder le playground actuel en §3.9.

### 3.3 Comment ça marche en 3 étapes

> **Eyebrow:** How it works
> **H2:** Three steps. Your coding agent can run them too.
>
> 1. **Pick the words.** Choose a group of keys, like a hero's title, subtitle and button, so they change together. Name the audiences: country, language, device, traffic source.
> 2. **Let AI write, let code check.** Choose intentions such as *social proof* or *simplicity*. glot writes each variant in every locale and refuses any that break a rule, with the exact reason. Nothing goes live until every locale passes.
> 3. **Ship the winner.** glot shows each audience the best version, waits for real evidence, then opens a pull request with the winner. Your catalogs stay the source of truth.
>
> **Footnote:** Experiment definitions live in `glot.experiments.toml`, next to your code. `glot experiments validate` checks them offline in CI.

### 3.4 Garanties

> **Eyebrow:** The guarantees
> **H2:** AI proposes. Code decides what visitors see.
> **Subtitle:** The same validator that guards glot's translations runs on every variant, in every locale. A variant with a single failing locale never goes live.

| Carte | Titre | Corps |
|---|---|---|
| 1 | Placeholders and tags | `{name}`, `<b>`, ICU, i18next and MessageFormat 2: same set as the source, none invented, none translated. |
| 2 | Plurals | Every form the language needs: four in Polish, six in Arabic. |
| 3 | Your glossary | Product names stay put. Required and forbidden terms per language, in a file in your repo. |
| 4 | Pixel width | Measured with your app's real font and box. A variant too wide for its button is refused, never cut off. |
| 5 | No dark patterns | Fake urgency, fake activity counters, unprovable claims and confirmshaming are refused. Prices and legal mentions can't change. |
| 6 | Only your facts | A variant can only cite numbers you declared. "Join 10,000 teams" is refused unless 10,000 is true and on your list. |

Chaque carte garde le pattern actuel « Refused by glot » + message réel. Microcopy carte 5 (petit texte) : *Rules target practices EU consumer law already sanctions. A clean report means no known red flag, not legal advice.* (repris de `policy.rs`, ne pas surpromettre la conformité).

### 3.5 Résultats par segment

> **Eyebrow:** Results you can read
> **H2:** "Better in Germany, 97% sure." Not a p-value.
> **Subtitle:** For every audience, glot says which version wins, by how much, and how sure it is, in one sentence.
>
> - **Plain sentences.** "Simplicity: +18% signups in DACH (probably between +6% and +30%), 97% probability it's the best. About 3 more days." *(example)*
> - **No early calls.** No winner before 7 days and 500 visitors per version, so one busy Monday never decides.
> - **No fake winners.** The bar rises with every look, version and audience. Our test suite simulates hundreds of experiments where nothing changes and fails if more than 5% of them declare a winner.
> - **Segment winners only when they're real.** glot says "winner for Germany" only if Germany clearly beats the global result. Otherwise it recommends promoting everywhere.
> - **Traffic goes to what works.** While learning, more visitors see the versions that look better. Every version keeps at least 10% until the end.
> - **Alarms you can't miss.** If the traffic split looks wrong (a cache, a bot, a bug), decisions freeze and a red banner tells you why.

- H2 « Not a p-value » : clin d'œil dev, compréhensible pour un non-statisticien grâce à la citation. Alternative plus sobre : « Results in plain words. »
- La phrase d'exemple **doit** porter « (example) » à l'écran, lisible (pas seulement en tooltip).
- Résultats : feux + texte (pas seulement couleur), cf. SC 1.4.1.

### 3.6 Livraison (sans flicker, sans cookie)

> **Eyebrow:** Fast and private by default
> **H2:** The right text in the first byte.
>
> - **No flicker.** Your edge picks the version, your server renders it. Visitors never see the old text swap out.
> - **Never on your critical path.** Decisions use a cached file refreshed every minute. If glot is down, everyone gets your official text.
> - **No cookie by default.** glot keeps counts per version and audience, per day. No IP, no user agent, no visitor ID. Global Privacy Control is respected.
> - **Search engines see what humans see.** Same URL, same canonical, never a version picked for crawlers.
>
> **Small print:** Single-page apps without server rendering can use the client-only mode, which may flicker.

### 3.7 Pour qui

> **Eyebrow:** Who it's for
> **H2:** Built for the people who ship the words.

| Persona | Titre | Corps |
|---|---|---|
| Dev / founder | You own the code | Three lines in your Next.js app. Your `t()` calls don't change. Winners arrive as pull requests you review like any other. |
| Growth / PM | You own the numbers | Launch a test on a hero in every language from the dashboard, or ask your coding agent. Read the result in one sentence. |
| Localization | You own the quality | Every variant passes your glossary and review queue. Nothing reaches a market you haven't approved. |

### 3.8 Intégrations

> **Eyebrow:** Integrations
> **H2:** Works with your stack and your agent.
>
> - **i18n:** next-intl, react-i18next, i18next, FormatJS, remix-i18next. ICU, i18next and MessageFormat 2 syntax.
> - **Frameworks:** Next.js (App and Pages Router), React Router, Astro, Vite + React.
> - **Edge:** Next.js proxy, Cloudflare Workers (static sites too), geo headers from Cloudflare, Vercel and CloudFront.
> - **Agents:** Claude Code, Cursor, Codex through the glot MCP server. Lovable, Bolt, v0 and Replit through a generated prompt.
> - **Git:** GitHub app, GitHub Action with PR annotations, `glot.experiments.toml` in your repo.
> - **Models:** your own key for Anthropic, OpenAI or any OpenAI-compatible endpoint, including local models.

Ne lister que ce qui est dans `frameworks.md`, `integrations.md`, `experiments.md`. Logos = logos de projets open source / outils, avec la mention « Works with », jamais « Trusted by ».

### 3.9 Socle i18n (« and of course… »)

> **Eyebrow:** The foundation
> **H2:** And yes: your whole app, in every language you ship.
> **Subtitle:** Variants are built on glot's localization engine. One command moves hardcoded strings to keys, translates only what changed, and refuses anything broken.
>
> `npx glot-i18n localize --yes --targets fr,de`
>
> **Proof:** In our benchmark, glot wrote **0.0%** broken strings. The same models with a plain prompt wrote **14.1%** (Haiku) and **8.8%** (Sonnet). An independent FormatJS judge agrees. → *Read the full benchmark*
> **Cost:** 850 strings cost $0.035 with the default model in our run. You pay your LLM provider directly.

Réutiliser ici, sans changement de fond, les blocs actuels `bench`, `fit` (« Translations that fit the button »), `play` (« Can you break glot? ») et `overlay`. Le comparatif (`compare`) reste, mais sous un lien « Compare with Lokalise, Crowdin, Tolgee, Lingo.dev » plutôt qu'en section pleine, pour raccourcir la page.

### 3.10 FAQ — objections

| Objection | Q | A |
|---|---|---|
| Qualité IA | Will AI-written copy embarrass us? | Every variant goes through the same checks as your translations, in every locale, plus rules against fake urgency, invented numbers and unprovable claims. You can require a human approval before any variant goes live. |
| Mensonge | Can a variant invent a stat or a discount? | No. A variant may only cite the facts you declare. Any number, percentage, price or date not in your text or your facts is refused. Keys tagged as pricing or legal can't be tested at all. |
| Perf | Does it slow my site down? | No blocking call. The decision runs at the edge from a cached file, the variant is in the first HTML, and exposures are sent after the response. |
| Panne | What if glot goes down? | Visitors get your official text and nothing is counted. The cached configuration keeps working for up to 24 hours. |
| RGPD / cookies | Do I need a cookie banner for this? | glot sets no cookie by default and stores no IP, user agent or visitor ID, only daily counts. An optional session cookie holds the version, nothing else. Whether you need consent is your DPO's call; our privacy notes help. |
| SEO | Is this cloaking? | No. glot never picks a version based on the user agent. Crawlers get the same URL and canonical as humans, and variants keep the same meaning. |
| Trafic | How much traffic do I need? | It depends on your traffic and conversion rate. When you create an experiment, glot estimates how long it will take, and suggests merging an audience that would need more than 90 days. Start with one global experiment. |
| Fiabilité stats | How do I know a winner is real? | glot waits at least 7 days and 500 visitors per version, raises the bar each time it looks, and keeps a small holdout if you want to measure everything you've promoted. |
| Lock-in | What happens to the winner? | It's written into your catalog files, in every locale, through a pull request. Stop using glot tomorrow and your app keeps the winning text. |
| Coût | What does it cost? | The CLI and validator are MIT and free. Experiments run on glot cloud (1 free experiment, segments on Pro and Team) or on your own server with no limits. |
| SPA | My app is a client-side SPA. | It works with the client-only mode, which may flicker on the first paint. Server rendering or a Cloudflare Worker avoids it. |

### 3.11 Pricing teaser

> **H2:** Free where it runs on your machine.
> **Body:** The CLI, the validator and the local MCP server are MIT and never limited. glot cloud starts free with one experiment. Pro adds audiences, Team adds more. Self-host the server and nothing is limited.
> **CTA:** **See pricing**

Reprend le titre actuel de `/pricing` (« Free where it runs on your machine. Paid where it runs on ours. ») tronqué. Pas de prix en dur (gérés par Stripe, page pricing fait foi).

### 3.12 CTA final

> **H2:** Ship the words that work, in every language.
> **Body:** Start with one headline. glot does the writing checks, the math and the pull request.
> **CTA 1:** `npx glot-i18n init` · **Copy the command**
> **CTA 2:** **Start free**
> **Tertiary link:** Read the experiments docs

---

## 4. Microcopy

| Emplacement | Copy |
|---|---|
| Nav | How it works · Guarantees · Results · Docs · Pricing · **Open the app** |
| Skip link | Skip to content |
| Copier la commande | Copy the command → (après clic, annoncé en `aria-live`) Command copied |
| Hint CTA SaaS | 1 experiment free. No credit card. *(⚠ à vérifier)* |
| Badge preuve benchmark | 0.0% broken vs 14.1% without glot |
| Étiquette des maquettes | Example data |
| Étiquette refus | Refused by glot |
| Étiquette accepté | Passed in every locale |
| Statut variante | Writing… · Checking 3 locales… · Ready · Refused: 1 reason |
| Résultat en cours | Still learning. About {days, plural, one {# more day} other {# more days}}. |
| Résultat gagnant | {variant}: {lift} {goal} in {segment}, {probability} probability it's the best. |
| Pas de gagnant | No clear winner yet. Your official text keeps running. |
| SRM | Traffic split looks wrong. Decisions are paused until you check. |
| Bouton promotion global | Promote everywhere |
| Bouton promotion segment | Promote for {segment} only |
| Vide (aucune expérience) | No experiments yet. Pick a headline and glot writes the first variants. |
| Erreur de chargement | Couldn't load results. Your visitors still see your official text. Try again. |
| Alt GIF démo | (reprendre `demoAlt` actuel pour la partie traduction) |
| Pause animation | Pause the demo / Play the demo |

Typographie anglaise : `18%`, `0.0%` sans espace (l'actuel `14.1 %` suit la typo française — à corriger dans `en.json` ; les autres locales gèrent leur format via glot).

---

## 5. Titres et métadonnées SEO

`docs/marketing/seo.md` n'existe pas encore : propositions à aligner quand il sera écrit (mots-clés à valider par volume).

| Page | `<title>` (≤ 60 car.) | Meta description (≤ 155 car.) |
|---|---|---|
| Home | glot: AI copy variants, safe in every language | AI writes variants of your copy. glot checks every locale, serves each country and device its best version, and sends the winner to your repo. Open source. |
| /docs/experiments | Segmented copy experiments · glot docs | Test headlines and buttons in every language, per country, device and source. No flicker, no cookie by default, winners promoted by pull request. |
| /docs/benchmark | AI translation benchmark: 0.0% vs 14.1% broken · glot | 320 ICU messages, 8 locales, same models: how many broken strings reach the catalog with and without glot. Reproducible. |
| /pricing | Pricing · glot | (inchangé) |

- H1 de la home = H1 du hero gagnant ; le `<title>` reste stable pendant l'expérience (les variantes ne touchent que `landing.hero.*`, pas `meta`).
- Requêtes cibles : « AI copywriting localization », « multilingual landing page optimization », « personalize website copy by country », « next-intl experiments », « i18n AI translation validation ». Éviter de cibler la requête portant le terme banni.
- OG image : H1 + « Example data » si elle montre un résultat.

---

## 6. Voix

- **Direct, concret, calme.** Verbes d'action, sujets humains (« your AI writes », « glot refuses »). Le sujet de « refuse » est toujours glot / le code.
- **Preuve > adjectif.** Aucun « powerful », « seamless », « best-in-class », « revolutionary ». Si un adjectif n'est pas mesurable, il part. (La règle `superlative` de glot s'appliquerait à notre propre page.)
- **Humour sec autorisé une fois par écran** (« Not a p-value »), jamais aux dépens de l'utilisateur.
- **Lisibilité :** phrases ≤ 20 mots, paragraphes ≤ 3 lignes, une idée par puce. Viser Flesch-Kincaid ≤ 8 sur la copy hors code.
- **Honnêteté sur les limites** dans la page même (SPA qui flicker, trafic nécessaire, avis DPO) : c'est un argument de confiance pour un public dev.

---

## 7. Checklist « aucune fausse preuve »

- [ ] Chaque nombre de la page figure au §1 avec sa source.
- [ ] « +18% / 97% » n'apparaît que dans une maquette marquée *Example data*.
- [ ] Aucun témoignage, logo « Trusted by », compteur d'utilisateurs, note ou classement.
- [ ] Étoiles GitHub et version npm lues en direct (déjà le cas dans `landing.tsx`), jamais en dur.
- [ ] « 30 languages » absent tant que non prouvé (§1).
- [ ] « No credit card » vérifié dans le flux Free.
- [ ] Le terme banni absent de `en.json` (grep en tête de doc) **et** des pages docs liées depuis la landing.

---

## 8. Vocabulaire retenu à la place du terme banni

| Au lieu de… | On écrit | Usage |
|---|---|---|
| le nom de la méthode (nom) | **experiment** / **copy experiment** | Nom générique, nav, docs (« Experiments ») |
| « tester A contre B » (verbe) | **try versions**, **put … to work**, **let visitors decide** | Corps de texte |
| variante, bras | **variant** (page), **version** (pour les non-techs) ; « arm » reste interne aux docs/API | |
| bras de contrôle | **your official text** | Partout sur la page |
| test classique à répartition fixe | **Fixed split** | Libellé du choix d'allocation (dashboard + `experiments.md`) |
| bandit / Thompson | **Optimize automatically** / **traffic goes to what works** | Page et dashboard ; « bandit » seulement dans la doc stats |
| significativité / p-value / posterior | **probability it's the best**, **probably between X and Y**, **how sure** | Résultats |
| segment / cellule | **audience** (page), **segment** (docs/API) | « Each audience gets its winner » |
| personnalisation | **serve the winner to {audience}** | Bouton « Promote for {segment} only » |
| holdout | **holdout** + glose « a few visitors who always see your official text » | FAQ, résultats |
| SRM | **traffic split looks wrong** | Bandeau |

Formule de catégorie pour parler du produit en une ligne : **« copy experiments for multilingual apps »** ou, plus long, **« AI-written copy, checked by code, served per audience »**.
