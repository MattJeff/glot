# GEO / AEO : faire citer glot par les moteurs IA et choisir par les agents de code

*Audit du 18 octobre 2026. Périmètre : glotlang.com, le repo `MattJeff/glot`, les paquets npm `glot-i18n` et `@glot-i18n/*`, `integrations/`. Aucune modification de code.*

## 1. Résumé en 5 lignes

1. **Aujourd'hui, glot n'existe pour aucun moteur.** Sur les 10 requêtes testées, il n'est cité nulle part. La requête « glot i18n glotlang » renvoie `@freddieridell/glot`, le glot Elixir, GlotPress et `glotctl`. Le nom est très encombré.
2. **Le site ne donne rien à lire aux robots.** `robots.txt`, `sitemap.xml`, `llms.txt`, `llms-full.txt` et les miroirs `.md` des docs renvoient tous 404 (vérifié par curl le 18/10/2026). Le site n'a aucun JSON-LD. Le dépôt GitHub n'a ni topics ni homepage, et il a 0 étoile.
3. **Le levier n°1 pour un devtool n'est pas le site.** Ce sont les endroits où l'agent cherche au moment de choisir : registre MCP, annuaire de plugins Claude Code, skills.sh, Context7, le README et la description npm. glot a déjà les fichiers (`integrations/`), mais ils ne sont publiés (ou trouvables) dans aucun de ces endroits. Le registre MCP ne renvoie rien pour `glot`.
4. **Les docs se contredisent sur des faits qu'un LLM va reprendre.** La FAQ dit « no hosted plan » alors que `/pricing` affiche Free/Pro/Team. Le format « JSON » est présenté comme seul support alors que le code gère `.xcstrings`, `.strings`, Android XML, ARB, PO et YAML. Les noms `@glot/*` (package.json) et `@glot-i18n/*` (npm, docs) coexistent.
5. **La niche « tester des textes par segment, dans toutes les langues » est vide.** Les moteurs y citent Seatext, Talkable, Tailor ou Tiny A/B Test, mais aucun outil dev/i18n. C'est la requête où glot peut devenir la réponse le plus vite.

## 2. Ce que disent les études (sources datées)

| Constat | Source | Fiabilité |
|---|---|---|
| Sur 252 000 essais et 6 LLM, quatre facteurs « gardiens » décident quelle source est citée en premier : **correspondance au sujet, prix, fraîcheur, position** dans le contexte récupéré. La complétude et les signaux de confiance comptent moins. La mise en forme a un effet négligeable. | [Vishwakarma et al., « What Gets Cited », arXiv 2605.25517 (mai 2026)](https://arxiv.org/pdf/2605.25517) | Preprint |
| Une revue critique de 2023 à 2026 conclut que la pertinence et la position dominent. Les recettes génériques se transfèrent mal, et réécrire un texte « pour être cité » peut **dégrader** sa récupération. Le recouvrement des sources d'un moteur à l'autre est faible, et les résultats varient d'une exécution à l'autre. | [Survey GEO 2023–2026, arXiv 2607.14035 (juillet 2026)](https://arxiv.org/html/2607.14035v1) | Preprint, revue |
| Optimiser seulement le corps du texte (SAGEO Arena) fait baisser de 6 % la citation finale. Il faut d'abord **être récupéré**. | idem | Preprint |
| Les gains de Princeton GEO (statistiques, citations, guillemets) ne valent que si la source est déjà dans le contexte. | [Aggarwal et al. 2024, discuté dans la survey](https://arxiv.org/pdf/2607.14035) | Académique |
| `llms.txt` : 36 120 fichiers en mai 2026 (×8,8 en un an). Pourtant, **97 % ne reçoivent aucune requête**. Les bots de réponse IA font 1,1 % des fetchs. ClaudeCode est le **2e** demandeur. Les outils de code (Cursor, Copilot, Claude Code) sont le vrai public. Google dit ne pas l'utiliser. | [Ahrefs via PPC Land](https://ppc.land/llms-txt-adoption-rises-8-8x-but-97-of-files-get-zero-ai-requests/), [Common Crawl, juillet 2026](https://commoncrawl.org/blog/a-content-analysis-of-llms-txt-files-from-the-july-2026-crawl-archive), [Olostep](https://www.olostep.com/blog/what-is-llms-txt) | Éditeurs, cohérents entre eux |
| Reddit est le domaine le plus cité par ChatGPT Search, Google AI Mode et Perplexity (Semrush, 325 000 prompts, janvier–février 2026 ; Peec, 30 M de citations, mars 2026). YouTube monte dans AI Overviews. Seuls 11 % des domaines sont cités à la fois par ChatGPT et Perplexity. Les parts de citation sont très volatiles. | [Peec](https://docs.peec.ai/research/most-cited-domains-in-ai-search), [eMarketer/Semrush](https://www.emarketer.com/chart/c/368764/reddit-top-cited-domain-across-chatgpt-search-google-ai-mode-perplexity-368764), [Whitehat SEO](https://whitehat-seo.co.uk/blog/ai-engines-comparison-citations) | Éditeurs |
| Les sources citées par ChatGPT et Perplexity sont environ 26 % plus récentes que les résultats SEO équivalents. La fraîcheur compte. | [LLMrefs](https://llmrefs.com/generative-engine-optimization) | Éditeur |
| Agents de code (étude Armature, 3 sept. 2026, 16 893 sessions) : Claude Code, Codex et Cursor ne choisissent le même service que dans **42 %** des catégories. Un agent ne « recommande » pas : il installe. Si le premier essai échoue, il passe au suivant sans le dire. | [andrew.ooo, 6 sept. 2026](https://andrew.ooo/answers/how-to-get-your-tool-picked-by-ai-coding-agents-2026-guide/) (source secondaire) | À vérifier |
| Context7 indexe un dépôt via `context7.com/add-library` ou un `context7.json` à la racine. Il lit `llms.txt` comme format d'entrée. | [Context7 docs](https://context7.com/docs/adding-libraries) | Officiel |
| Annuaire de plugins Claude : soumission par formulaire, revue automatique, badge « Anthropic Verified » après une revue supplémentaire. Distinguer `claude-plugins-official` du marketplace communautaire. | [claude.com/docs/plugins/submit](https://www.claude.com/docs/plugins/submit), [code.claude.com/docs/en/discover-plugins](https://code.claude.com/docs/en/discover-plugins) | Officiel |

**Ce que j'en retiens pour glot :**

- **Être dans le contexte récupéré avant de soigner la prose.** Cela passe par une correspondance exacte à la requête (H1 et premières lignes formulés comme la question), la fraîcheur (dates visibles) et la présence sur les domaines que les moteurs récupèrent déjà : GitHub, Reddit, npm, registres d'agents.
- **Le prix est un facteur gardien.** Il faut écrire « gratuit, MIT, vous payez seulement votre clé LLM, 850 chaînes = 0,035 $ » dans les 100 premiers mots de chaque page clé.
- **`llms.txt` sert les agents de code, pas ChatGPT.** C'est peu coûteux, à faire, sans en attendre de miracle.

## 3. Test de 10 requêtes (18 octobre 2026)

Méthode : recherche web (moteur de recherche de l'outil, US) en reprenant la synthèse de type « AI answer » qu'il produit. C'est une approximation de ChatGPT Search et Perplexity, pas un test direct de chaque assistant (voir la section 8 pour le suivi réel).

| # | Requête | Cités / recommandés | glot ? |
|---|---|---|---|
| 1 | best i18n tool for Next.js with AI translation | Locize (blog avril 2026), Better i18n (MCP + `@better-i18n/next`), Intlayer, next-intl | Non |
| 2 | how to translate my Lovable app | react-i18next (guide générique), Globalize.now (skill Lovable + Lingui), Lovalingo, Localize, Common Ninja | Non |
| 3 | localize SwiftUI app with AI (xcstrings) | Xlocalize, String Studio (open source), AI Localizer, PTC (WPML), skills `ios-localization` / `axiom-localization` | Non (alors que glot lit `.xcstrings`) |
| 4 | test copy variants per country | Seatext, Talkable, Tailor AI, Tiny A/B Test, Localazy (définition) | Non |
| 5 | MCP server for i18n (Claude Code / Cursor) | **Lingo.dev** (pages dédiées `/i18n-mcp/claude-code`, `/cursor`, `/codex`), SimpleLocalize, mcp-i18n, KAERIS | Non |
| 6 | open source alternative to Lokalise / Crowdin with LLM, git-based | Tolgee, Weblate, GitLocalize, Lingo.dev, Languine (via openalternative.co et alternativeto.net) | Non |
| 7 | validate AI translations placeholders ICU plurals CI | Aucun outil : WebTranslateIt (blog), frontendchecklist.io, puis « écrivez votre script » | Non, **niche vide** |
| 8 | add i18n to my Next.js app with Claude Code | Skills communautaires : openclaudia `i18n` (`npx skills add …`), plusieurs skills next-intl (openskillindex, claudeskills.info, skills-hub) | Non |
| 9 | detect translated text overflow (German button too long) | LocaleProof (Figma), LocaleGuard (Unreal), AutoControl, Playwright fait maison | Non, **niche quasi vide** pour le web |
| 10 | translate Flutter ARB with AI CLI | arb_ai, Lingo.dev CLI (page `/cli/formats/flutter`), arb_translator_gen_z | Non |
| bonus | glot i18n glotlang | @freddieridell/glot, glot (Elixir), GlotPress, glotctl | **Non : collision de nom** |

**Lecture :**

- **Lingo.dev** gagne parce qu'il a **une page par couple outil × intention** (`/i18n-mcp/claude-code`, `/cli/formats/flutter`). C'est le modèle à copier.
- Les annuaires de skills (skills.sh, openskillindex, claudeskills.info) sont **déjà cités** par les moteurs sur les requêtes d'agents. glot a un `SKILL.md` qui n'y figure pas.
- Les requêtes 4, 7 et 9 n'ont pas de bonne réponse aujourd'hui. Une page canonique bien titrée peut y passer en tête.

## 4. Constats sur le repo et le site

| Point | État | Effet |
|---|---|---|
| `glotlang.com/robots.txt`, `/sitemap.xml` | 404 | Crawl et découverte des pages de docs ralentis, pas de signal de fraîcheur |
| `/llms.txt`, `/llms-full.txt`, `/docs/*.md` | 404 | Claude Code, Cursor et Context7 doivent parser du HTML Next |
| JSON-LD (`SoftwareApplication`, `FAQPage`, `Organization`) | absent | Les moteurs lisent moins bien le prix, la licence, l'OS et le lien vers le repo |
| GitHub `MattJeff/glot` | description OK, **0 topic, pas de homepage**, 0 étoile | Les topics `i18n` et `mcp-server` alimentent GitHub search et les annuaires |
| Registre MCP (`io.github.MattJeff/glot`) | `server.json` prêt, **non trouvé** par l'API `?search=glot` | L'agent qui cherche « i18n MCP » ne voit pas glot |
| Plugin Claude Code (`integrations/claude-code/glot`) | prêt, non soumis à l'annuaire | Absent de `/plugin` et des recherches sur les plugins |
| Skill (`integrations/skills/glot/SKILL.md`) | bonne description, pas sur skills.sh | Absent des annuaires que les moteurs citent (requête 8) |
| `smithery.yaml`, `glama.json` | présents | Vérifier que les fiches sont publiées et à jour en 0.4.0 |
| npm `glot-i18n` | description « AI-native i18n for SaaS… », 8 keywords, homepage = GitHub | Il manque `localization`, `translate`, `next.js`, `react`, `xcstrings`, `android`, `flutter`, `arb`, `mcp-server`, `claude-code`, `cursor`, `llm`, `ci`. La homepage devrait être glotlang.com |
| Docs `faq.md` | « no hosted plan » | Contredit `/pricing` (Free/Pro/Team). Le LLM répétera l'une ou l'autre version |
| Formats mobiles | gérés dans `crates/core/src/formats/` (xcstrings, strings, android, arb, po, yaml) et `examples/mobile-formats`, **absents des docs** | glot ne peut pas être cité sur les requêtes 3 et 10 |
| Noms de paquets | `@glot/*` dans `packages/*/package.json`, `@glot-i18n/*` dans les docs et sur npm | Un agent qui lit la source installera un paquet qui n'existe pas |
| `experiments.md` | très bon contenu factuel (SSR sans flicker, pas de cloaking, GPC/DNT, SRM) | Le H1 (« Segmented experiments ») ne reprend aucune formulation de requête utilisateur |
| Landing `meta.description` | « 0 % broken strings… against 14.1 % » | Bon chiffre sourcé. La nouvelle landing doit garder un chiffre de ce type dans sa description |

## 5. Vingt actions priorisées

Légende : **Effort** S (moins d'une heure), M (une demi-journée), L (plus). **Impact** : agents de code (A), moteurs de réponse (R), les deux (A+R).

### P0 : à faire cette semaine

1. **Corriger les contradictions factuelles (S, A+R).** Dans `web/content/docs/faq.md`, remplacer « no hosted plan » par « The CLI and MCP server are free and MIT. glot cloud (dashboard, review, experiments, GitHub sync) has a free plan and paid Pro/Team plans: glotlang.com/pricing. » Aligner `packages/*/package.json` sur `@glot-i18n/*`. Un LLM qui cite une incohérence cite aussi un concurrent.
2. **Publier `robots.txt` et `sitemap.xml` (S, R).** Fichiers `web/app/robots.ts` et `web/app/sitemap.ts` (conventions Next). Le sitemap liste toutes les docs × locales avec `lastModified` = date du dernier commit du `.md`. Le `robots.txt` autorise explicitement `GPTBot`, `OAI-SearchBot`, `ChatGPT-User`, `ClaudeBot`, `Claude-User`, `Claude-SearchBot`, `PerplexityBot`, `Perplexity-User` et `Google-Extended`, et pointe vers le sitemap.
3. **Publier `/llms.txt`, `/llms-full.txt` et `/docs/<slug>.md` (S–M, A).** Servir le markdown brut de `web/content/docs/*.md` (déjà la source de rendu) via un route handler. `llms-full.txt` = concaténation des docs dans l'ordre de `NAV`. Contenu de `llms.txt` proposé en section 6.
4. **Publier le serveur MCP au registre officiel (S, A).** `mcp-publisher publish` avec `integrations/registries/server.json`, puis vérifier `registry.modelcontextprotocol.io/v0/servers?search=glot`. Mettre `websiteUrl` à `https://glotlang.com/docs/mcp`. Ajouter `"mcpName": "io.github.MattJeff/glot"` dans `packages/npm-glot/package.json` si ce n'est pas déjà fait (le registre l'exige pour valider le paquet npm).
5. **Soumettre le plugin Claude Code (S, A).** Passer `integrations/claude-code/glot` par le formulaire de l'annuaire ([claude.com/docs/plugins/submit](https://www.claude.com/docs/plugins/submit)). En attendant, documenter dans le README l'installation via un marketplace git : `/plugin marketplace add MattJeff/glot` puis `/plugin install glot@glot`. Ajouter au `plugin.json` les keywords `localization`, `translation`, `multilingual`, `experiments`.
6. **Remplir la fiche GitHub (S, A+R).** Homepage `https://glotlang.com`. Topics : `i18n`, `l10n`, `localization`, `translation`, `mcp-server`, `claude-code`, `cursor`, `next-intl`, `i18next`, `react-i18next`, `xcstrings`, `android`, `flutter`, `llm`, `rust`, `cli`, `ai-translation`, `copy-testing`. Description de 150 caractères max : « Open-source CLI + MCP server: your coding agent localizes your app, deterministic checks refuse broken or too-long translations. MIT, BYO LLM key. »
7. **Réécrire `description` et `keywords` npm (S, A).** Dans `packages/npm-glot/package.json` : `"description": "Localize any app with your coding agent: extract strings, AI-translate, and refuse broken placeholders, plurals or overflowing text. CLI + MCP server, MIT."`. Keywords : ajouter ceux de l'action 6, plus `translate`, `nextjs`, `react`, `vite`, `ci`, `github-action`. `homepage` → `https://glotlang.com`. Même passe sur chaque `@glot-i18n/*`, avec un README d'au moins 10 lignes et un exemple copiable par paquet.

### P1 : ce mois-ci

8. **Context7 (S, A).** Soumettre `MattJeff/glot` sur `context7.com/add-library`, et ajouter un `context7.json` à la racine : `folders: ["web/content/docs", "integrations"]`, `excludeFolders: ["target", "crates", "bench", "web/app"]`. Dans `rules`, mettre les 5 règles d'`integrations/agents/AGENTS.md`.
9. **skills.sh et annuaires de skills (S, A+R).** Rendre le skill installable par `npx skills add MattJeff/glot --skill glot` : le layout `skills/<name>/SKILL.md` est attendu à la racine ou sous `integrations/skills/`, à tester. Le faire indexer sur skills.sh, puis vérifier openskillindex et claudeskills.info (cités sur la requête 8).
10. **Documenter les formats mobiles (M, A+R).** Nouvelle page `web/content/docs/mobile.md` (« Localize an iOS, Android or Flutter app with AI ») : une section par format (`.xcstrings` / SwiftUI, `.strings`, Android `strings.xml`, Flutter `.arb`, gettext `.po`, YAML), chacune avec un `glot.toml` copiable et un exemple tiré de `examples/mobile-formats`. Ajouter une ligne « Formats » au tableau Guarantees du README.
11. **Une page par intention, sur le modèle Lingo.dev (M, R).** Créer sous `/docs` (ou `/guides`) des pages courtes. Le H1 reprend la requête et une réponse en 2 phrases suit, puis la commande :
    - « Add i18n to a Next.js app with Claude Code » (MCP + plugin + skill)
    - « Translate a Lovable / Bolt / v0 app » (à partir d'`integrations/ai-builders.md`, déjà rédigé)
    - « Localize a SwiftUI app with AI » (renvoie à l'action 10)
    - « Validate AI translations in CI: placeholders, ICU plurals, glossary » (`glot check`, GitHub Action)
    - « Catch translated text that overflows its button » (`glot measure` / `fit`)
    - « Test copy variants per country, in every language » (expériences)
12. **Repositionner la page expériences pour la requête 4 (S, R).** Dans `experiments.md`, mettre en H1 « Test copy variants per country and segment, in every language », et en première phrase : « glot serves several versions of a headline or call to action to the segments you choose (country, device, traffic source), in every locale, rendered server-side with no flicker, and keeps the one that converts. » Ajouter une ligne de prix et un tableau « glot vs outils de test de copy », sourcé comme `docs/comparison-sources.md`.
13. **JSON-LD (S, R).** Sur la landing : `SoftwareApplication` (`applicationCategory: DeveloperApplication`, `operatingSystem: macOS, Linux, Windows`, `offers.price: 0`, `license: MIT`, `codeRepository`, `softwareVersion` dynamique). Ajouter `FAQPage` sur le bloc FAQ (section 7) et `Organization` avec `sameAs` vers GitHub, npm et le registre MCP. Sur les docs : `TechArticle` avec `dateModified`.
14. **Dates visibles partout (S, R).** Afficher « Updated 2026-10-18 » en tête de chaque page de docs (date git) et sur le tableau comparatif. La fraîcheur est un facteur gardien, et `comparison-sources.md` a déjà les dates.
15. **Lever l'ambiguïté du nom (S, R).** Écrire de façon constante « **glot (glotlang)** » ou « glot i18n » dans le `<title>`, la description npm, le H1 du README et le JSON-LD (`alternateName: ["glotlang", "glot-i18n"]`). Viser l'entité `glotlang` plutôt que le mot `glot`, déjà pris par 4 projets.

### P2 : présence hors site (les domaines que les moteurs citent)

16. **Reddit, de façon utile et non promotionnelle (M, R).** Les brouillons existent (`docs/launch/reddit.md`). Au-delà du lancement : répondre avec une solution réelle (commande + sortie) aux fils r/nextjs, r/reactjs, r/iOSProgramming, r/FlutterDev, r/lovable, r/SaaS qui demandent « how to translate my app » ou « i18n with AI ». Mentionner glot comme une option parmi d'autres, avec un lien vers la doc précise. Pas de faux comptes : les moteurs recoupent leurs sources (voir les pièges listés par andrew.ooo).
17. **Show HN + GitHub Discussions + Stack Overflow (M, R).** Lancer le Show HN (`docs/launch/show-hn.md`). Ouvrir les Discussions avec 5 Q&R canoniques (Lovable, SwiftUI, CI, overflow, copy par pays). Sur Stack Overflow, répondre aux questions `next-intl` / `i18next` sur la validation des pluriels ICU avec un vrai snippet `glot check`.
18. **Annuaires d'alternatives (S, R).** Fiches glot sur openalternative.co (pages « alternatives to Lokalise / Crowdin », citées sur la requête 6), alternativeto.net, Glama et Smithery (vérifier la 0.4.0), awesome-mcp-servers, awesome-i18n. PR vers les listes « awesome » existantes.
19. **Une page benchmark citable, avec des chiffres sourcés (M, R).** `web/content/docs/benchmark.md` existe. Le chiffre « 0 % de chaînes cassées contre 14,1 % sans glot » doit avoir son protocole, sa date, le modèle utilisé et un lien vers le script, et une URL stable (`/benchmark`). Publier un fichier de résultats en CSV dans le repo. C'est le type de contenu qu'un moteur cite comme preuve, et que des blogs tiers reprennent.
20. **Suivi mensuel (S, A+R).** Reprendre chaque mois les requêtes de la section 8 dans ChatGPT (search on), Perplexity, Gemini, Google AI Mode, Copilot et Claude (web). Lancer aussi 3 sessions Claude Code et Cursor dans un repo Next.js vide avec « add i18n to my app in French and German », et noter ce que l'agent installe. Journaliser dans `docs/marketing/geo-log.csv` (date, moteur, requête, cité oui/non, rang, concurrents). Ne pas optimiser pour un seul agent (accord de 42 % seulement).

## 6. `llms.txt` proposé (`https://glotlang.com/llms.txt`)

```markdown
# glot (glotlang)

> glot is an open-source (MIT) localization CLI and MCP server for coding agents. It moves hardcoded UI strings to keys, translates them with your own LLM key (Anthropic, OpenAI, or any OpenAI-compatible endpoint, including local models), and refuses any translation that breaks a placeholder, misses a CLDR plural form, violates the glossary or is wider than its UI box. Catalogs stay plain files in git; no runtime dependency is added. glot cloud adds a dashboard, review, GitHub sync and copy experiments per segment.

Key facts:
- License: MIT. CLI and MCP server are free; you pay only your LLM provider (850 strings cost $0.035 with the default model and `--batch` in our run; `glot translate --dry-run` estimates first).
- Install: `npx glot-i18n localize --yes --targets fr,de` (init → i18nize → translate → check, idempotent). Also Homebrew (`brew install mattjeff/glot/glot`) and a shell installer.
- Libraries: next-intl, react-i18next / i18next, remix-i18next, FormatJS (ICU, i18next and MessageFormat 2 syntax).
- File formats: JSON (nested/flat), Apple String Catalog (.xcstrings) and .strings, Android strings.xml, Flutter ARB, gettext PO, YAML.
- Automatic i18nize (hardcoded strings → keys): Next.js App Router and Pages Router, React Router 7/8, Astro, Vite + React.
- Agents: MCP server (`npx -y glot-i18n mcp`, registry name io.github.MattJeff/glot), Claude Code plugin, Agent Skill, AGENTS.md / Cursor / Windsurf rules.
- Source of truth: git. `glot.lock` translates only the delta and never overwrites a hand edit without `--force`.

## Docs
- [Install & quickstart](https://glotlang.com/docs/getting-started.md): install, `init`, `i18nize`, `translate`, try without a key
- [Concepts](https://glotlang.com/docs/concepts.md): validator, lockfile, glossary, pixel width
- [CLI reference](https://glotlang.com/docs/cli.md): every command and flag
- [Frameworks](https://glotlang.com/docs/frameworks.md): what `i18nize` writes per stack
- [MCP server](https://glotlang.com/docs/mcp.md): tools exposed to Claude Code, Cursor, Codex
- [Agent skill](https://glotlang.com/docs/skills.md): install the glot skill / Claude Code plugin
- [Integrations & GitHub Action](https://glotlang.com/docs/integrations.md): CI check, PR comments, AI builders (Lovable, Bolt, v0, Replit)
- [Quality](https://glotlang.com/docs/quality.md): checks run on every translation
- [Copy experiments per segment](https://glotlang.com/docs/experiments.md): test several versions of a headline or CTA per country, device or traffic source, in every locale, server-rendered
- [FAQ & costs](https://glotlang.com/docs/faq.md): pricing, providers, privacy, cost control

## Comparisons
- [Benchmark](https://glotlang.com/docs/benchmark.md): broken strings with and without glot, protocol and data
- [Comparison sources](https://github.com/MattJeff/glot/blob/main/docs/comparison-sources.md): glot vs Lokalise, Crowdin, Lingo.dev, Tolgee, each cell sourced and dated

## Optional
- [Full docs in one file](https://glotlang.com/llms-full.txt)
- [Runtime (on-device long-tail locales)](https://glotlang.com/docs/runtime.md)
- [Self-hosting](https://glotlang.com/docs/self-hosting.md)
- [Labs](https://glotlang.com/docs/labs.md): SEO, database content, emails/images, task parity
- [GitHub repository](https://github.com/MattJeff/glot)
- [npm: glot-i18n](https://www.npmjs.com/package/glot-i18n)
```

Ne publier la ligne « File formats » qu'une fois `mobile.md` en ligne (action 10), et retirer les liens `.md` tant que l'action 3 n'est pas faite.

## 7. Bloc FAQ factuel pour la landing (à baliser en `FAQPage`)

Contrainte : ne pas écrire « A/B testing ». Réponse directe dans la première phrase, chiffres datés, 60 mots maximum par réponse.

**What is glot?**
glot is an open-source (MIT) CLI and MCP server that lets your coding agent localize your app. It extracts hardcoded strings, translates them with your LLM key, and refuses any translation that breaks a placeholder, misses a plural form, violates your glossary or overflows its UI box.

**How much does glot cost?**
The CLI and MCP server are free and MIT-licensed. You pay your LLM provider directly: 850 strings cost $0.035 with the default model in our run (October 2026). `glot translate --dry-run` estimates the cost first, and a budget can cap it. glot cloud has a free plan and paid team plans.

**Can I test different copy for different countries?**
Yes. glot serves several versions of a headline or call to action to the segments you choose (country, device, traffic source, plan), in every language at once, and keeps the one that converts. The version is chosen at the edge and rendered by the server, so visitors never see the text change.

**Does showing different copy hurt SEO?**
No. glot never chooses a version based on the user agent. Search engines get a random version, like any visitor, on the same URL with the same canonical. Bot visits are not counted.

**Is it privacy-friendly?**
By default no cookie is set, and Global Privacy Control or Do Not Track visitors get the official text and are not counted. Only the name of the matched segment reaches glot, never raw attributes like IP or country.

**Which frameworks and file formats does glot support?**
Automatic extraction and wiring: Next.js (App and Pages Router), React Router 7/8, Astro and Vite + React. Catalog formats: JSON with ICU, i18next or MessageFormat 2 syntax, Apple .xcstrings and .strings, Android strings.xml, Flutter ARB, gettext PO and YAML.

**Which coding agents work with glot?**
Any agent that speaks MCP (Claude Code, Cursor, Codex, Windsurf), through `npx -y glot-i18n mcp`. Claude Code also has a glot plugin and skill, and `AGENTS.md` rules are provided for other agents.

**Which AI models does glot use?**
Your own: Anthropic, OpenAI, or any OpenAI-compatible endpoint, including local models through Ollama. The default is a small, cheap model. A stronger fallback model is used only when a string fails validation twice.

**Does glot add a runtime dependency?**
No. Your app keeps next-intl, i18next or FormatJS with plain catalog files in git. Runtime packages are opt-in (copy experiments, on-device fallback, in-app editing).

**How is glot different from Lokalise, Crowdin or Lingo.dev?**
Git is the source of truth, every LLM output is checked by deterministic code before it is written, your coding agent gets live context through MCP, and translations are measured against the pixels they actually have. Every cell of our comparison links to a public, dated source.

## 8. Requêtes à suivre (mensuel, cf. action 20)

Moteurs : ChatGPT (search), Perplexity, Gemini, Google AI Mode / AI Overviews, Copilot, Claude (web). Agents : Claude Code et Cursor sur un repo vierge.

**Copy par segment (nouvelle landing)**
1. test copy variants per country
2. show different headline per country website Next.js
3. localized landing page copy testing tool
4. personalize website copy by traffic source without flicker
5. how to test translated headlines in multiple languages

**i18n + IA (cœur)**

6. best i18n tool for Next.js with AI
7. AI translation for next-intl / react-i18next
8. open source alternative to Lokalise / Crowdin / Lingo.dev
9. i18n MCP server for Claude Code / Cursor
10. how to add i18n to my app with Claude Code

**AI builders**

11. how to translate my Lovable app
12. make my Bolt / v0 / Replit app multilingual

**Mobile**

13. localize SwiftUI app with AI (xcstrings)
14. translate Android strings.xml with AI
15. translate Flutter ARB files with AI

**Qualité (niches vides, quick wins)**

16. validate AI translations placeholders ICU plurals in CI
17. detect text overflow after translation German
18. glossary enforcement for LLM translation

**Marque**

19. glot i18n / glotlang / what is glot localization
20. glot vs Lingo.dev / glot vs Tolgee

**Test agent (prompt exact, repo Next.js vierge)**

- « Add French and German to this app. »
- « Translate this app and make sure nothing breaks in CI. »
- « Show a different hero headline to visitors from Germany. »

Ce qu'on note à chaque fois : l'agent a-t-il trouvé glot (par le MCP, une recherche ou un skill installé), quelle commande a-t-il lancée, le premier run a-t-il réussi.

## Sources

- [Vishwakarma et al., What Gets Cited (arXiv 2605.25517)](https://arxiv.org/pdf/2605.25517)
- [Critical Survey of GEO 2023–2026 (arXiv 2607.14035)](https://arxiv.org/html/2607.14035v1)
- [CITECHOICE (arXiv 2609.15164)](https://arxiv.org/pdf/2609.15164)
- [PPC Land / Ahrefs, llms.txt 97 % zero requests](https://ppc.land/llms-txt-adoption-rises-8-8x-but-97-of-files-get-zero-ai-requests/)
- [Common Crawl, llms.txt July 2026](https://commoncrawl.org/blog/a-content-analysis-of-llms-txt-files-from-the-july-2026-crawl-archive)
- [Olostep, what is llms.txt](https://www.olostep.com/blog/what-is-llms-txt)
- [Peec, most cited domains in AI search](https://docs.peec.ai/research/most-cited-domains-in-ai-search)
- [eMarketer, Reddit top-cited domain](https://www.emarketer.com/chart/c/368764/reddit-top-cited-domain-across-chatgpt-search-google-ai-mode-perplexity-368764)
- [Whitehat SEO, AI engines citation comparison](https://whitehat-seo.co.uk/blog/ai-engines-comparison-citations)
- [LLMrefs, GEO guide 2026](https://llmrefs.com/generative-engine-optimization)
- [How to get your tool picked by AI coding agents (sept. 2026)](https://andrew.ooo/answers/how-to-get-your-tool-picked-by-ai-coding-agents-2026-guide/)
- [Context7, adding libraries](https://context7.com/docs/adding-libraries)
- [Claude plugin directory, submit](https://www.claude.com/docs/plugins/submit)
- [Claude Code, discover plugins](https://code.claude.com/docs/en/discover-plugins)
- [Lingo.dev i18n MCP for Claude Code](https://lingo.dev/en/i18n-mcp/claude-code)
- [Lingo.dev CLI Flutter ARB](https://lingo.dev/en/cli/formats/flutter)
- [openalternative.co, Lokalise alternatives](https://openalternative.co/alternatives/lokalise)
- [Globalize.now Lovable skill](https://globalize-now.lovable.app/)
- [Seatext, combining A/B testing with localization](https://seatext.seatext.com/guides/how-to-combine-a-b-testing-with-localization-6a7de4b4)
- [openclaudia i18n skill](https://claudeskills.info/skills/openclaudia/openclaudia-skills/i18n/)
