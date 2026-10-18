# L'i18n que produisent les builders IA, et ce que glot doit en faire

Recherche du 2026-10-18/19. Trois sources :

- (a) la doc officielle et les forums, lus le 2026-10-18 ;
- (b) un échantillon GitHub mesuré avec `gh search code` sous un compte authentifié : 360 dépôts Lovable tirés au hasard, plus 97 Bolt et 97 v0 ;
- (c) des tests de `target/integ/debug/glot` sur des projets factices dans le scratchpad.

Les constats sur glot correspondent à l'état du repo à cette date.

**Légende :** **[preuve]** = observé dans le code ou dans une doc officielle ; **[inférence]** = déduit, non vérifié.

## TL;DR

1. **Aucun builder ne fixe de méthode i18n par défaut.** Ni Lovable, ni Bolt, ni v0, ni Replit, ni Firebase Studio, ni base44 n'en documentent une. L'IA en choisit une à chaque prompt.
   - Chez Lovable, **55 %** des apps multilingues utilisent i18next (react-i18next), et **45 %** un système fait main. **[preuve, n=53]**
   - Chez Bolt et v0, le fait main domine : 5 apps sur 9 chez Bolt, 3 sur 5 chez v0. **[preuve, petits n]**
2. Le pattern fait main dominant est **un objet `translations = { en: {...}, fr: {...} }` dans un fichier TS**. Il est consommé par `LanguageContext.tsx` et `useLanguage().t(key)`. Dans 6 cas sur 15, le dictionnaire se trouve dans le fichier du contexte lui-même.
3. **glot rate aujourd'hui une bonne partie de ces patterns.** Dans le pire cas, `i18nize` installe un second runtime react-i18next à côté du contexte existant (§3.2).
4. Depuis le **13 mai 2026**, les nouveaux projets Lovable sont en **TanStack Start** (rendu serveur), et non plus en SPA Vite. Sur ce stack, `glot i18nize` ne convertit pas le texte des routes `createFileRoute({ component: () => ... })` et ne trouve pas de point d'entrée (§3.3).
5. **La convention glot n'est pas imposée par `glot init`.** Le bloc écrit dans CLAUDE.md, SKILL.md et `.cursor` ne parle ni de nommage des clés, ni de la syntaxe des placeholders et des pluriels, ni du hook à utiliser, ni du sélecteur de langue. Le texte riche existe pourtant déjà (`glot agent-prompt`), mais `init` ne l'utilise pas (§4).

## 1. Stack par défaut de chaque builder

| Builder | Stack par défaut | Ce qui sort quand on demande « make it multilingual » | Sources |
|---|---|---|---|
| **Lovable** | Nouveaux projets : **TanStack Start** (SSR) depuis le 13/05/2026, et le 22/06/2026 pour Enterprise. Avant : Vite + React + TS + shadcn + Tailwind + Supabase. Les anciens projets restent en SPA. | Deux familles **[preuve, §2]** : react-i18next avec ressources importées en dur, ou contexte fait main. Le guide officiel ne nomme aucune librairie. Il suggère de demander un sélecteur de langue, des chemins `/es/` et des balises hreflang. | docs.lovable.dev/features/upgrade-to-tanstack-start ; lovable.dev/en/blog/building-apps-using-tanstack-start (01/06/2026) ; lovable.dev/en/guides/how-to-build-multilingual-website (30/01/2026) |
| **Bolt.new** | Vite + React en général ; Next, Astro et SvelteKit sont possibles ; Expo pour le mobile. | Fait main ×5, i18next JSON ×3, i18next + fichiers TS ×1, sur 9 apps multilingues parmi 97 dépôts **[preuve]**. Pas de doc officielle. | expo.dev/blog/bolt-expo-integration-announcement ; échantillon §2 |
| **v0** | Next.js App Router + shadcn | Fait main ×3, i18next ×1, next-intl ×1, sur 5 apps parmi 97 **[preuve]**. La preview de v0 casse avec next-intl et react-intl (forum Vercel, mai 2025, confirmé en février 2026) ; le conseil donné est i18next + un menu déroulant shadcn. Cela explique **[inférence]** que v0 évite next-intl. | community.vercel.com/t/internationalization-in-v0-dev/10157 |
| **Replit Agent** | React + Vite + Tailwind + shadcn + Express + Postgres. L'Agent 3 impose React même quand on demande du JS natif. | Pas de doc. Les guides tiers recommandent react-i18next. Le comportement attendu est celui de Lovable SPA **[inférence]**. | docs.replit.com/teams/custom-templates ; replit.discourse.group/t/…/7030 |
| **Firebase Studio** | Next.js (+ Firestore, Genkit). Preview ; la création de nouveaux workspaces est fermée. | Pas de doc i18n. Probablement next-intl ou un dictionnaire maison **[inférence]**. | firebase.google.com/docs/studio/get-started-ai |
| **base44** | React + React Router + Vite + shadcn, avec `@base44/sdk`. Le code est **exportable** (sync GitHub, eject). | Pas de doc. L'écran de login intégré est en anglais seulement (feature request « In Review »). | docs.base44.com/…/project-structure ; feedback.base44.com/p/feature-request-multilingual-support-for-the-built-in-login-screen |
| **Cursor, Claude Code, Windsurf, Copilot** | Pas de stack propre : ils suivent le projet. | Ils adoptent ce qui existe déjà. Sur un projet vierge, les skills communautaires suivent la règle : next-intl pour App Router, react-i18next pour Vite, clés `home.title` **[inférence + skills publics]**. | productbuilder.net/zh/skills/internationalization ; skills-hub.ai/skills/i18n |

**Concurrents qui ciblent Lovable** (lus le 2026-10-18) :

- **Globalize.now**, skill `lovable-i18n` (13/06/2026, mis à jour 07/09) : installe **Lingui + catalogues `.po`** et dit explicitement « avoid inventing a custom translation context ». Leur article du 24/04/2026 décrit le problème central : chaque prompt Lovable réintroduit du texte en dur.
- **i18.dev** : FormatJS.
- **Lovalingo**, **Localize.js**, **Common Ninja** : traduction au runtime, sans fichiers.
- **i18nexus** : react-i18next + cloud.
- Aucun ne lit **en place** un dictionnaire fait main : c'est un créneau libre pour glot.

## 2. Ce que l'IA génère vraiment (échantillon GitHub)

**Méthode.**

- Le pool vient de `gh search code "lovable-tagger filename:package.json path:/ size:<plage>"` sur 8 plages de taille, soit 640 dépôts distincts.
- On a tiré 2 échantillons aléatoires de 200 dépôts ; 360 avaient un `package.json` lisible.
- Les sources ont été téléchargées par tarball. Seuls les `.ts/.tsx/.js/.jsx/.json` ont été scannés (lus comme texte, jamais exécutés). Chaque dépôt signalé a ensuite été classé à la main.
- Ordre de grandeur côté GitHub : environ 78 300 `package.json` contiennent `lovable-tagger`, et environ 2 860 d'entre eux ont aussi `i18next` (3,7 %).

**Limites.**

- L'échantillon est biaisé vers les dépôts que la recherche GitHub classe le mieux.
- Forks et doublons n'ont pas été retirés.
- Environ 74 dépôts sans signal n'ont pas été re-vérifiés après des téléchargements tronqués. Le taux d'erreur observé sur les autres est faible : 2 manqués sur 277.
- Pour Bolt et v0, on n'a lu que la liste des fichiers et le `package.json` : les ternaires dans le code n'y sont pas détectés.

**Combien d'apps sont multilingues :** 53 sur 360 chez Lovable (**14,7 %**), environ 9 sur 97 chez Bolt (9 %), environ 5 sur 97 chez v0 (5 %).

### Répartition des 53 apps Lovable multilingues

| # | Pattern | n | % | Emplacements observés |
|---|---|---|---|---|
| A | react-i18next + JSON importés en dur | 21 | 40 % | `src/i18n/locales/{lng}.json` ×8 · `src/locales/{lng}/translation.json` ×5 · `src/locales/{lng}.json` ×5 · `src/i18n/locales/{lng}/{ns}.json` ×3 · `src/i18n/resources/{lng}.json` ×1 · `public/locales/…` + http-backend ×1 |
| C | **Dictionnaire TS unique + contexte maison** | 15 | 28 % | dans `src/contexts/LanguageContext.tsx` (ou `context/`, `hooks/useLanguage.tsx`) ×6 · `src/i18n/translations.ts` ×3 · `src/lib/translations.ts` ×2 · `src/utils/translations.ts` ×2 · `src/lib/translations.js` ×1 · `contexts/language-context.tsx` ×1 |
| B | i18next + ressources en TS | 7 | 13 % | inline dans `src/i18n.ts`, `src/i18n/config.ts` ou `src/lib/i18n.ts` ×3 · un fichier TS par langue (`src/i18n/translations/{lng}.ts`, `src/locales/{lng}/common.ts`…) ×4 |
| D | Un fichier fait main par langue | 4 | 8 % | `src/translations/{lng}.js`, `src/locales/{lng}.ts`, `src/i18n/locales/{lng}/{ns}.ts` |
| F | Inline seulement, sans clés | 4 | 8 % | `t('English', 'Bangla')`, `tr({ en: '…', pt: '…' })`, plus de 150 ternaires `language === 'ta' ? … : …`, un dictionnaire dans chaque composant |
| — | Contexte maison + JSON par namespace | 1 | 2 % | `src/i18n/locales/{lng}/{ns}.json` |
| — | A ou B, structure non vérifiée | 1 | 2 % | |
| E | next-intl / next-i18next | 0 | 0 % | absent, car Lovable produit une SPA |

**Observations transverses** (toutes **[preuve]**) :

- **Ternaires :** 7 des 15 apps C ont **aussi** des ternaires `language === 'fr' ? … : …`, de 5 à 276 par dépôt. Le fait main n'est presque jamais « propre ».
- **Clés :**
  - A et B : JSON imbriqué lu avec un chemin pointé (`nav.home`, `common.cancel`) presque partout.
  - C : plates et pointées `'nav.home'` ×7, camelCase plat ×4, snake_case ×1, objet imbriqué lu comme propriété `t.hero.title` sans fonction ×2.
  - **Jamais la phrase elle-même comme clé.**
- **Typage C :** `type TranslationKey = keyof typeof translations.en` ×6, `Record<Language, Record<string, string>>` ×2, `interface Translations` ×2.
- **t() fait main :** `translations[lang][key] || key`, parfois avec un repli `|| translations.en[key]`.
  - Interpolation `{name}` par `.replace` dans seulement 4 dépôts faits main.
  - **Aucun pluriel** dans le fait main. Côté i18next, `_one`/`_other` dans 4 dépôts.
- **Persistance :**
  - C : `localStorage` avec une clé maison (`'<app>-lang'`) dans 7 cas sur 15.
  - A et B : `i18next-browser-languagedetector` dans environ 17 cas sur 29.
  - `document.documentElement.lang` ou `dir='rtl'` est défini dans 6 à 8 dépôts.

### Structures types (anonymisées)

**C1, dictionnaire dans le contexte** (la forme faite main la plus fréquente) :

```tsx
type Language = 'fr' | 'en';
const LanguageContext = createContext<LanguageContextType | undefined>(undefined);
const translations: Record<Language, Record<string, string>> = {
  fr: { 'nav.home': 'Accueil', 'hero.title': '…' },
  en: { 'nav.home': 'Home',    'hero.title': '…' },
};
export const LanguageProvider = ({ children }) => {
  const [language, setLanguage] = useState<Language>(() => (localStorage.getItem('<app>-lang') as Language) || 'en');
  const t = (key: string) => translations[language][key] || key;
  …
};
export const useLanguage = () => { const c = useContext(LanguageContext); if (!c) throw new Error('…'); return c; };
```

**C2, fichier séparé avec clé typée :**

```ts
// src/lib/translations.ts
export type Language = 'en' | 'hi' | 'ta';
export const translations = { en: { nav_home: 'Home' }, hi: { … }, ta: { … } };
export type TranslationKey = keyof typeof translations.en;
// LanguageContext.tsx
const t = (key: TranslationKey) => translations[language][key] || translations.en[key] || key;
```

**A, i18next avec ressources importées en dur** (`src/i18n/config.ts` ou `src/i18n/index.ts`) :

```ts
import LanguageDetector from 'i18next-browser-languagedetector';
import en from './locales/en.json'; import fr from './locales/fr.json';
i18n.use(LanguageDetector).use(initReactI18next).init({
  resources: { en: { translation: en }, fr: { translation: fr } }, fallbackLng: 'en',
});
```

**F, paires inline :**

```ts
const t = (en: string, bn: string) => (language === 'en' ? en : bn);
<span>{t('Subtotal', 'সাবটোটাল')}</span>
```

## 3. Ce que glot fait aujourd'hui sur ces patterns, et ce qu'il doit faire

### 3.1 Détection par `glot init` (testée le 2026-10-18)

| Layout (et part dans l'échantillon) | Résultat aujourd'hui | Verdict |
|---|---|---|
| A `src/i18n/locales/{lng}.json`, `src/locales/…`, `{lng}/{ns}.json` (l’essentiel de A) | détecté | OK |
| A `src/i18n/resources/{lng}.json` (≈2 %) | **raté** : retombe sur `public/locales/{locale}/common.json` | glot va écrire de nouveaux catalogues vides à côté des vrais |
| B inline dans `src/i18n/config.ts` ou `src/lib/i18n.ts` (≈6 %) | **raté** : même repli (seuls `src/i18n.{js,ts}` et `src/i18n/index.*` sont lus) | idem |
| B ou D, un fichier TS ou JS par langue dans un dossier nommé i18n / translations / locales (≈4 B + 4 D) | détecté (`TS/JS dictionaries`) | OK |
| **C dictionnaire dans `translations.ts`** (≈15 %) | **raté** : stack `react-i18next`, catalogue `src/locales/{locale}.json` | **grave**, voir 3.2 |
| **C dictionnaire dans `LanguageContext.tsx`** (≈13 %) | **raté**, et même configuré à la main : `parse error … expected value` (`.tsx` n'est pas mappé au format Js) | **grave** |
| F inline seulement (≈8 %) | vu comme du texte en dur | acceptable, c'est une migration |
| TanStack Start (tous les nouveaux Lovable) | `react (no i18n library yet)` | voir 3.3 |

**La lecture et l'écriture en place fonctionnent déjà.** Avec `files = ["src/i18n/translations.ts"]` (forme C2, `as const`), `glot status` lit les deux locales. `glot translate` ajoute ensuite un bloc `de: {…}` imbriqué dans le même objet sans rien casser d'autre. Il faut juste **brancher la détection**, et traiter deux défauts : l'union `type Language = 'en' | 'fr'` n'est pas mise à jour, et l'extension `.tsx` n'est pas acceptée.

### 3.2 Le pire cas : `i18nize` sur un contexte maison non détecté

Sur le projet C1, `glot i18nize --dry-run` installe i18next, crée `src/i18n.ts` et produit ceci :

```tsx
const { t: tr } = useTranslation(); const { t } = useLanguage();
… <h1>{t('hero.title')}</h1> … <p>{tr("hero.no_credit_card_required")}</p>
```

Le composant se retrouve avec deux runtimes, deux sélecteurs de langue désynchronisés, et les traductions FR existantes ignorées. Pour quelqu'un qui débarque avec une app Lovable, c'est rédhibitoire.

Quand le dictionnaire **est** détecté, `i18nize` refuse proprement (« this app has its own i18n runtime… pass --lib to migrate »). Ce comportement est bon, mais il ne convertit rien.

Effet de bord observé : `glot extract` remonte les **valeurs du catalogue TS lui-même** comme « config strings » (`i18n.translations.build_faster_title`). Ce sont des faux positifs, car les fichiers du catalogue ne sont pas exclus de l'extraction.

### 3.3 TanStack Start (le nouveau défaut Lovable)

Projet minimal : `@tanstack/react-start`, `src/routes/__root.tsx`, `src/routes/index.tsx` avec `createFileRoute('/')({ component: () => <h1>Welcome…</h1> })`.

- `i18nize` répond : « 1 strings left as is: not inside a React component ». Le composant inline des routes n'est pas reconnu, alors que c'est **le** pattern de TanStack Router.
- Le câblage s'arrête sur « note: no src/main.tsx: add `import "./i18n";` to your entry ». Le bon point d'entrée serait `src/routes/__root.tsx` ou `src/router.tsx`.
- Le SSR est mieux géré : `detect()` est protégé par `typeof window`. Mais côté serveur la langue retombe sur la source : il faut lire le préfixe d'URL dans le loader, sinon le rendu serveur et le client divergent (hydratation).
- TanStack recommande officiellement Paraglide (tanstack.com/router/latest/docs/guide/internationalization-i18n). On ne sait pas encore ce que Lovable génère sur ce stack : **à mesurer dans 2–3 mois**, quand les dépôts TanStack Start seront assez nombreux.

### 3.4 Ce que glot doit faire, par pattern

Principe : **adopter sans réécrire par défaut**, et proposer la migration en option. Le fondateur Lovable veut « traduire mon app », pas changer de librairie.

| Priorité | Pattern | Détection `init` | Adoption sans réécriture | Extraction / i18nize | Migration en option |
|---|---|---|---|---|---|
| **P0** | **C** dictionnaire TS + `useLanguage` | Chercher dans `src/**` un fichier `.ts/.tsx/.js` dont le premier objet de haut niveau a **au moins 2 clés de locale** (`en`, `fr`…) dont les valeurs sont des objets. Indices : un nom contenant `translation`, `i18n`, `Language`, `locale`, ou un `createContext` + `useLanguage` voisin. Stack = `custom (LanguageContext)`, syntaxe = `{name}` si `.replace(` est vu, sinon pas de placeholder. | Déjà possible via le format Js. À ajouter : **mapper `.tsx/.jsx`** sur `Format::Js` ; **mettre à jour l'union `type Language = …`** et un éventuel tableau `languages = [...]` ou les options du sélecteur quand une locale est ajoutée. | **Mode `--lib custom`** : réécrire le texte en dur avec le `t` existant (`const { t } = useLanguage()`, import repris d'un composant qui l'utilise déjà) et écrire la clé dans le dictionnaire source. Exclure les fichiers du catalogue de l'extraction. | `glot i18nize --lib react-i18next --from custom` : génère `src/locales/*.json` depuis le dictionnaire, remplace `useLanguage().t` par `useTranslation().t` (même signature `t(key)`), et garde `useLanguage()` comme façade qui délègue à i18n, pour ne pas toucher le sélecteur de langue. Codemod en un seul diff. |
| **P0** | **A** react-i18next + JSON | Ajouter : lire `resources` et les `import … from './…/xx.json'` dans `src/i18n/**`, `src/lib/i18n.*` et `src/i18n/config.*`, et prendre le chemin des imports plutôt que deviner un dossier. **Ne jamais retomber sur `public/locales/{locale}/common.json` sans un warning** quand i18next est présent. | OK | OK | — |
| **P0** | **TanStack Start** | Détecter `@tanstack/react-start` comme framework à part entière. | — | Reconnaître `component: () => <…/>` et `component: function X()` dans `createFileRoute` et `createRootRoute`. Câbler dans `src/routes/__root.tsx` ou `src/router.tsx`. Langue côté serveur via le préfixe d'URL ou un cookie. | — |
| **P1** | **B** i18next + TS inline | Même lecture des imports et de `resources`, appliquée aux chemins `src/i18n/config.ts` et `src/lib/i18n.ts`. | OK via le format Js | OK | — |
| **P1** | **F** ternaires et paires inline (aussi présents dans la moitié des C) | `extract` : reconnaître `lang === 'xx' ? 'A' : 'B'` et `t('A', 'B')` comme **une seule chaîne déjà traduite**. | — | Réécrire en `t('key')` en **reprenant la traduction existante** dans les deux catalogues (pas de retraduction LLM ; marquer `reviewed` dans `glot.lock`). | — |
| **P2** | **D** un fichier par langue | déjà détecté | OK | même mode `custom` que C | — |
| **P2** | next-intl (v0, Firebase) | déjà détecté | OK | OK | — |
| — | Lingui `.po` (Globalize) | 5 dépôts sur environ 78 000 | rien à faire maintenant | | |

## 4. Les règles écrites par `glot init` imposent-elles la méthodologie ?

Test : `glot init --yes --provider mock` sur un Vite + React factice (`lovable-tagger`, pas d'i18n), dans `scratchpad/ai-i18n/plain`.

Fichiers générés : `glot.toml`, `glossary.toml`, `.mcp.json`, `CLAUDE.md`, `.claude/skills/glot/SKILL.md`, `.agents/skills/glot/SKILL.md` (identique), `.cursor/rules/glot.mdc` (`alwaysApply: true`), `.github/workflows/glot.yml`. **Pas d'`AGENTS.md`** : il n'est mis à jour que s'il existe déjà (`w.block("AGENTS.md", &block, false)`).

Le bloc commun (`agent_block`, `crates/cli/src/init.rs`) :

```
- Write user-facing text only in the `en` catalog, never hardcoded in components.
- Before adding a key, call the MCP tool `propose_string` … Without MCP: `glot add <key> "<text>" --note "<context>"`.
- Never edit target catalogs (fr, es, de) by hand: run `glot translate` …
- Run `glot check` before finishing; it must pass.
```

| Exigence | Présente ? |
|---|---|
| Pas de texte en dur | oui |
| `glot check` obligatoire | oui |
| Ne pas éditer les catalogues cibles | oui |
| **Clés pointées par écran ou composant** (`billing.plan_card.upgrade_button`) | **non** (seulement dans `glot agent-prompt`) |
| **Quel hook et quel import utiliser** (`useTranslation`, `useTranslations("section")`, ou le `useLanguage` maison) | **non** |
| **Syntaxe des placeholders et des pluriels du stack** (ICU, ou `{{x}}` + `_one`/`_other` pour i18next) | **non** |
| Pas de concaténation, Intl pour les dates et nombres, +30–40 % de longueur, marques non traduites | **non** (seulement dans `agent-prompt`) |
| « Ajoute une langue / rends l'app multilingue » → `glot localize`, **sans créer de LanguageContext** | **non** : c'est précisément le prompt qui produit le pattern C |
| Sélecteur de langue : utiliser `setLocale` de `src/i18n.ts` | **non** |
| Couverture des agents | Claude Code, Codex/Copilot (via `.agents/skills`), Cursor. **Manquent :** `AGENTS.md` quand il est absent (lu par Codex, Copilot coding agent, Windsurf, Cursor **[inférence, selon leurs docs 2025-2026]**) et `replit.md`. Lovable, Bolt et v0 ne lisent pas les fichiers de règles : il faut passer par `glot agent-prompt --for lovable` à coller dans Knowledge. |

**Incohérences.**

- `glot agent-prompt` contient déjà presque tout le bon texte (`crates/cli/src/localize.rs::agent_prompt`), mais `init` ne s'en sert pas : deux sources de vérité.
- Pour le stack `TS/JS dictionaries`, `agent_prompt` tombe dans la branche `_` et dit « No i18n library yet… install react-i18next ». C'est faux et dangereux sur une app C : l'agent installerait un deuxième runtime.
- La ligne « Placeholders and ICU plurals » est affichée même pour i18next, dont la syntaxe n'est pas ICU.
- La description du SKILL généré (« Add, change or translate user-facing strings… ») ne contient pas les mots qui déclenchent le skill (« multilingual », « add a language », « language switcher »). La version portable `integrations/skills/glot/SKILL.md` les a.

### Améliorations proposées (texte exact)

**1. Une seule source.** `agent_block()` devient `MARK_START + "## Internationalization (glot)\n\n" + agent_prompt(root, "agent") + ligne MCP + MARK_END`. Pour react-i18next, cela donne :

```markdown
<!-- glot:start -->
## Internationalization (glot)

This app is localized with glot. Source locale: `en`. Targets: fr, es, de. Catalogs: `src/locales/{locale}.json`.

- Every user-facing string (headings, buttons, labels, placeholders, aria-labels, titles, toasts, errors, empty states, emails) goes through `const { t } = useTranslation()` (`<Trans>` for markup). Never hardcode text in components, never `lang === 'fr' ? … : …`.
- Keys are nested and named `<screen>.<component>.<role>`, lowercase snake_case: `billing.plan_card.upgrade_button`, `auth.login_form.email_label`. Reuse `common.*` for generic actions (`common.cancel`, `common.save`).
- Add a string: MCP tool `propose_string` (server `glot`), then `add_string`. Without MCP: `glot add <key> "<text>" --note "<where it shows, max length>"`. Write only the `en` catalog; never edit fr, es, de: glot translates them.
- Placeholders `{{name}}`, plurals with `_one`/`_other` suffixes and `t("key", { count })`. Never build sentences by concatenation. Dates, numbers, currencies: `Intl` with the active locale.
- Leave room for text 30-40% longer than English: no fixed widths on buttons, tabs, badges.
- "Make the app multilingual" / "add a language": do NOT create a LanguageContext, a `translations` object or a new i18n library. Run `npx -y glot-i18n localize --yes --targets <locales>`. Language switcher: call `setLocale(locale)` from `src/i18n.ts`.
- Before finishing: `glot check` must exit 0. Commit `glot.lock` with the catalogs.
<!-- glot:end -->
```

Variante **next-intl** de la première puce : « … goes through `const t = useTranslations("<screen>")` (client) or `const t = await getTranslations("<screen>")` (server) … ». Et pour les pluriels : « ICU: `{count, plural, one {# invoice} other {# invoices}}` with `t("invoices.count", { count })`. Placeholders `{name}`. »

Variante **dictionnaire maison** (stack détecté `custom`), qui remplace la branche `_` actuelle :

```markdown
- This app has its own i18n runtime: `const { t } = useLanguage()` from `src/contexts/LanguageContext.tsx`, dictionary `src/i18n/translations.ts`. Keep using it: do NOT install i18next or next-intl, do NOT create a second dictionary. Add keys to the `en` object only, in the existing key style (`nav.home`); glot fills the other locales in place.
- Replace any `language === 'xx' ? … : …` you touch with `t('<key>')`.
```

**2. Description du SKILL** (`SKILL_HEAD`) :

```yaml
description: Localize this app with glot. Use when adding or changing any user-facing text, when asked to make the app multilingual, translate it, add a language or a language switcher, or when touching translation catalogs or glot.toml. Never hand-roll a translations object or LanguageContext.
```

**3. Couverture.**

- `w.block("AGENTS.md", &block, true)` : créer `AGENTS.md`, lu par Codex, Copilot, Windsurf et Cursor.
- `w.block("replit.md", &block, false)` : compléter `replit.md` s'il existe.
- À la fin de `init`, si `lovable-tagger`, `.bolt/` ou `components.json` + v0 est détecté, afficher : « Lovable/Bolt/v0 ne lisent pas ces fichiers : collez `glot agent-prompt --for lovable` dans Project settings → Knowledge. »

**4. Vérification.** Ajouter à `glot check` un warning `hand-rolled-i18n` quand un nouveau `const translations = { en: …, fr: … }` ou un `language === '` apparaît en dehors des catalogues déclarés. Comme le workflow CI tourne sur chaque PR, cela impose la règle même aux agents qui ignorent CLAUDE.md.

## 5. Classement final : quoi supporter en priorité

1. **Dictionnaire TS + `LanguageContext`/`useLanguage` (C, +D)** : 36 % des apps Lovable multilingues, majoritaire chez Bolt et v0, aujourd'hui raté et cassé par `i18nize`. → détection, `.tsx`, union `Language`, `i18nize --lib custom`, migration en option vers react-i18next.
2. **react-i18next avec ressources importées en dur (A+B)** : 53 %. Majoritairement détecté ; il faut lire les imports et `resources` plutôt que deviner les dossiers, et supprimer le repli silencieux vers `public/locales`.
3. **TanStack Start** : le défaut de tous les nouveaux projets Lovable depuis le 13/05/2026. Il faut gérer les composants inline des routes et le câblage dans `__root.tsx` / `router.tsx`.
4. **Ternaires et paires inline (F)** : 8 % seuls, mais présents dans la moitié des apps C. → extraction qui **reprend la traduction existante**.
5. **Règles agent** : réutiliser `agent_prompt` dans `init` (clés `<screen>.<component>.<role>`, syntaxe du stack, « pas de LanguageContext », `setLocale`), créer `AGENTS.md`, plus le warning `hand-rolled-i18n` dans `glot check`.
6. next-intl (v0, Firebase) : déjà géré. Lingui `.po` : à surveiller.

Les données brutes de l'échantillon (listes de dépôts, signaux, notes par dépôt) sont dans le scratchpad de la session (`gh-sample/`), sans les sources téléchargées.
