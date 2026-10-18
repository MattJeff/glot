# glot — Brief UX/UI (v2, 2026-10-18)

> Source de vérité partagée par les agents **onboarding**, **Connect GitHub**, **design system + dashboard**, **marketing**.
> Stack : `web/` Next.js 16.4 · React 19.3 · Tailwind v4 · shadcn/ui (radix-ui) · next-intl (en, fr, de, es, ja, **ar = RTL**) · sonner · cmdk · lucide-react · tw-animate-css.
> Objectif fondateur : **simple, rapide, animé, accessible, descriptif, intuitif, clair, propre, beau, gamifiant.**
> Règle d'or : *chaque animation, badge ou chiffre doit aider l'utilisateur à finir sa traduction plus vite. Sinon, on le supprime.*

Fichiers existants à réutiliser avant d'écrire quoi que ce soit : `web/app/globals.css` (tokens), `web/components/ui/*` (shadcn), `web/components/health-score.tsx`, `web/components/job-row.tsx`, `web/components/common.tsx`, `web/lib/snippets.ts`, `web/lib/use-error-message.ts`.

---

## 0. TL;DR (si tu ne lis qu'une section)

1. **Time-to-first-value < 3 min** : signup → projet créé → commande copiable avec token pré-rempli → écran qui *attend en direct* le premier push → célébration → « ajoute une langue ».
2. **Checklist de setup persistante** (4–5 étapes max) dans la sidebar jusqu'à complétion, puis disparaît.
3. **Gamification = progrès réel** : anneau % par langue, jalons liés à la qualité (0 erreur de validation, 1re langue à 100 %), badge README partageable. Jamais de points arbitraires, jamais de streak punitive.
4. **Motion** : 150–250 ms, `ease-out` en entrée, sortie plus rapide, CSS d'abord (tw-animate-css est déjà là), View Transitions pour la navigation, `motion-reduce:` partout.
5. **A11y WCAG 2.2 AA** : focus visible, tout au clavier, `aria-live` pour jobs/attente, cibles ≥ 24 px (44 conseillé), RTL via propriétés logiques (`ms-`, `pe-`, `start-`), axe + Playwright en CI.
6. **Copy** : verbe + objet (« Créer le projet »), erreurs = cause + action, description sous chaque champ non évident. Toutes les chaînes passent par next-intl.

---

## 1. Benchmarks onboarding devtools (ce qu'on copie)

| Produit | Pattern | Ce que glot en fait |
|---|---|---|
| **Vercel** | Import Git → build live avec logs qui défilent → confettis + screenshot du déploiement. Valeur visible en < 1 min. | Après « Connect GitHub », montrer le **scan du repo en direct** (fichiers de messages trouvés, nb de clés, langues détectées), puis la 1re traduction. |
| **Stripe** | Clés API *déjà* dans les snippets de doc quand on est connecté ; mode test par défaut. | `web/lib/snippets.ts` : toujours injecter le vrai token/slug du projet dans les commandes. Bouton Copier à droite de chaque bloc. |
| **Resend** | Onboarding = 1 page : API key → snippet → « Send test email » → l'écran passe au vert quand le 1er email arrive. | **Écran d'attente live** : « En attente de votre premier `glot push`… » avec pulse discret, polling/SSE, bascule automatique en succès. |
| **Supabase / Clerk** | Quickstarts par framework (Next, Vite, Rails…) en onglets ; Clerk affiche « Waiting for first sign-in ». | Onglets **CLI · GitHub Action · MCP (agent IA)** ; détecter le framework si possible et présélectionner l'onglet. |
| **Linear** | Onboarding court, raccourcis enseignés dès le départ, ⌘K omniprésent, animations très sobres et rapides. | Palette ⌘K (cmdk déjà installé), indices `kbd` dans les tooltips. |
| **PostHog** | Checklist « Get started » avec % de complétion, états vides qui expliquent quoi faire et montrent le snippet. | Checklist sidebar + états vides pédagogiques (voir §1.3). |
| **Raycast** | Micro-feedback instantané (toasts courts, HUD), tout au clavier. | Toasts sonner ≤ 4 s, action « Annuler » sur les opérations destructives. |
| **Duolingo** | Progression visible, jalons, streak, célébrations… mais aussi critiques sur la pression des streaks. | On prend la **progression visible + jalons + célébration**, on laisse la culpabilisation. |

### 1.1 Flux cible (onboarding + Connect GitHub)

```
Signup (OAuth GitHub en 1er bouton)
  → Nom du projet (prérempli depuis le repo) + langue source + langues cibles (chips, suggestions)
  → Choix du chemin : [Connect GitHub ✓ recommandé] [CLI] [Agent IA / MCP]
  → Commande copiable avec token prérempli  ──┐
  → Écran "En attente du premier push…" (live) ◄┘  (aria-live, bouton "Je ne vois rien ?" après 30 s)
  → Succès : "142 clés détectées · 3 langues" + anneaux qui se remplissent
  → CTA suivant unique : "Traduire en Français (142 clés)"
```

Règles :
- **Une seule action primaire par écran.** Le reste est en `variant="ghost"` ou lien.
- **Toujours un échappatoire** : « Passer pour l'instant » + la checklist garde l'étape.
- **Ne jamais demander ce qu'on peut deviner** (nom de projet depuis le repo, langue source depuis les fichiers).
- **Persister la progression** côté serveur (pas seulement localStorage) : on reprend où on s'est arrêté.

### 1.2 Checklist de setup (sidebar / haut du dashboard)

5 étapes max, chacune = un verbe + un résultat. Exemple :
1. Créer le projet ✓
2. Connecter GitHub (ou installer la CLI)
3. Pousser les messages source
4. Traduire une première langue
5. Ajouter le badge au README

```tsx
// Progress déjà dans components/ui/progress.tsx
<section aria-labelledby="setup-title" className="rounded-xl border bg-card p-4">
  <div className="flex items-center justify-between gap-2">
    <h2 id="setup-title" className="text-sm font-medium">{t("setup.title")}</h2>
    <span className="text-xs text-muted-foreground tabular-nums">{done}/{steps.length}</span>
  </div>
  <Progress value={(done / steps.length) * 100} aria-label={t("setup.progress", { done, total: steps.length })} className="mt-2 h-1.5" />
  <ol className="mt-3 space-y-1">
    {steps.map((s) => (
      <li key={s.id}>
        <Link href={s.href} aria-current={s.current ? "step" : undefined}
          className="flex min-h-9 items-center gap-2 rounded-md px-2 text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring">
          {s.done
            ? <CircleCheck className="size-4 text-success" aria-hidden />
            : <Circle className="size-4 text-muted-foreground" aria-hidden />}
          <span className={s.done ? "text-muted-foreground line-through" : ""}>{s.label}</span>
          <span className="sr-only">{s.done ? t("setup.done") : t("setup.todo")}</span>
        </Link>
      </li>
    ))}
  </ol>
</section>
```

### 1.3 États vides qui enseignent

Structure fixe : **icône (aria-hidden) · titre = ce qui manque · 1 phrase = pourquoi c'est utile · action primaire · (option) snippet copiable · lien doc.**

```tsx
<div className="flex flex-col items-center gap-3 rounded-xl border border-dashed p-10 text-center">
  <Languages className="size-8 text-muted-foreground" aria-hidden />
  <h3 className="text-base font-medium">{t("empty.locales.title")}</h3>        {/* "Aucune langue cible" */}
  <p className="max-w-sm text-sm text-muted-foreground">{t("empty.locales.body")}</p> {/* "Ajoutez une langue : glot traduit vos 142 clés et valide les placeholders." */}
  <Button>{t("empty.locales.cta")}</Button>                                     {/* "Ajouter une langue" */}
</div>
```

### 1.4 Commande copiable

- Bloc `font-mono`, fond `bg-muted`, bouton Copier `aria-label` explicite, feedback icône Check 1,5 s + `aria-live="polite"` « Copié ».
- Token **masqué visuellement par défaut** quand affiché hors onboarding (`glot_••••abcd`), mais **copié en entier**. Pendant l'onboarding, montré une fois avec un avertissement « Gardez-le secret ».
- Une commande par ligne, pas de `$` copié.

### 1.5 Attente live du premier événement

```tsx
<div role="status" aria-live="polite" className="flex items-center gap-3 rounded-lg border p-4">
  <span className="relative flex size-2.5" aria-hidden>
    <span className="absolute inline-flex size-full animate-ping rounded-full bg-brand opacity-60 motion-reduce:hidden" />
    <span className="relative inline-flex size-2.5 rounded-full bg-brand" />
  </span>
  <p className="text-sm">{received ? t("wait.received", { keys }) : t("wait.waiting")}</p>
</div>
```
- Polling via TanStack Query `refetchInterval: 2000` (s'arrête dès réception) — pas besoin de WebSocket pour la v1.
- Après 30 s sans rien : afficher une aide contextuelle (« Vérifiez que `GLOT_TOKEN` est défini », lien diagnostic). Jamais de timeout bloquant.

### 1.6 Célébrations

- **Rares et méritées** : 1er push reçu, 1re langue à 100 %, 0 erreur de validation, badge ajouté. Pas à chaque clic.
- Forme : anneau qui se remplit + check animé + toast. Confettis **une seule fois** (1re langue à 100 %), désactivés si `prefers-reduced-motion`. Pas de lib si possible ; si on en veut une : `canvas-confetti` (~3 kB) appelée avec `disableForReducedMotion: true`.
- Le message dit **ce qui a été accompli et la suite** : « Français : 142/142 clés ✓ — Ajoutez l'allemand ? »

---

## 2. Gamification qui sert (pas du gadget)

Principe : on gamifie **la complétude et la qualité des traductions**, qui sont exactement ce que veut l'utilisateur. Cadre : **théorie de l'autodétermination (SDT)** — nourrir *compétence* (progrès visible, feedback), *autonomie* (ordre libre des étapes, opt-out), *relation* (partage, équipe) ; éviter les leviers « perte/rareté » manipulateurs.

Règles dures :
- **Barres exactes** : jamais de progression gonflée (« 20 % offerts » au départ). Un dev le voit et perd confiance.
- **Score transparent** : chaque point du score de santé est explicable (« −12 : 3 placeholders manquants en DE »).
- **Opt-out** : tout est masquable sans perte de fonction.

### 2.1 Mécaniques retenues

| Mécanique | Données réelles | UI |
|---|---|---|
| **Anneau de progression par langue** | % clés traduites + validées | Anneau SVG 32–48 px + `tabular-nums` « 87 % » + drapeau/code ISO (jamais drapeau seul : langue ≠ pays). |
| **Score de santé projet** (existe : `health-score.tsx`) | couverture, erreurs de validation (placeholders/ICU), clés obsolètes, glossaire respecté | Note 0–100 + **liste « Comment gagner des points »** cliquable : chaque item = une action qui corrige. |
| **Jalons** | événements réels | « Premier push », « Première langue à 100 % », « 0 erreur de validation », « 5 langues », « 1 000 clés traduites », « Agent IA connecté (MCP) », « Badge README ajouté ». Affichés dans une page/onglet *Progress*, pas en popup bloquant. |
| **Streak raisonnable** | semaines (pas jours) avec ≥1 sync | « 4 semaines d'affilée en sync » — **hebdo**, opt-out, jamais de notif culpabilisante, gel automatique en vacances. Les critiques de Duolingo (anxiété liée à l'aversion à la perte, streak qui prime sur l'apprentissage) montrent le risque. **Optionnel, à livrer en dernier, ou pas du tout.** |
| **Badge README** | % de couverture | `https://app.glotlang.com/badge/<slug>.svg` style shields (« i18n 94 % · 6 langues »). C'est le levier de partage/viralité n°1 pour un devtool. |
| **Delta visible** | diff depuis la dernière visite | « +38 clés traduites depuis hier » en haut du dashboard. |

### 2.2 Anneau de progression (sans dépendance)

```tsx
export function ProgressRing({ value, label, size = 40 }: { value: number; label: string; size?: number }) {
  const r = (size - 4) / 2, c = 2 * Math.PI * r;
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={label} className="-rotate-90 rtl:rotate-90 rtl:-scale-x-100">
      <circle cx={size/2} cy={size/2} r={r} fill="none" strokeWidth="4" className="stroke-muted" />
      <circle cx={size/2} cy={size/2} r={r} fill="none" strokeWidth="4" strokeLinecap="round"
        strokeDasharray={c} strokeDashoffset={c * (1 - value / 100)}
        className={cn("transition-[stroke-dashoffset] duration-700 ease-out motion-reduce:transition-none",
          value === 100 ? "stroke-success" : "stroke-brand")} />
    </svg>
  );
}
// label = t("locale.progress", { locale: "Français", value: 87 }) → "Français : 87 % traduit"
```

### 2.3 Anti-patterns interdits

- ❌ Points/XP sans signification, classements entre utilisateurs/équipes.
- ❌ Streak quotidienne, notifications « Vous allez perdre votre série ! », compte à rebours, FOMO.
- ❌ Modales de célébration qui bloquent le travail ; confettis répétés.
- ❌ Ton condescendant (« Bravo champion ! 🎉🎉 »), emoji comme icônes.
- ❌ Gamifier ce que l'utilisateur ne contrôle pas, ou récompenser la quantité au détriment de la qualité (ex. « 1000 clés » alors que 30 % ont des erreurs → le jalon exige 0 erreur bloquante).
- ❌ Bloquer des fonctionnalités derrière des jalons.
- ✅ Tout élément gamifié doit être **désactivable** dans Settings (« Masquer les célébrations ») et lisible en lecteur d'écran.

---

## 3. Motion

### 3.1 Échelle (à mettre dans `globals.css`)

```css
@theme {
  --ease-out-quint: cubic-bezier(0.22, 1, 0.36, 1);   /* entrées */
  --ease-in-out-quart: cubic-bezier(0.76, 0, 0.24, 1); /* déplacements */
  --animate-pop: pop 300ms var(--ease-out-quint);
  @keyframes pop { 0% { transform: scale(.9); opacity: 0 } 60% { transform: scale(1.04) } 100% { transform: scale(1); opacity: 1 } }
}
```

| Usage | Durée | Easing |
|---|---|---|
| Hover, press, toggle | 100–150 ms | `ease-out` |
| Tooltip, popover, dropdown | 150 ms entrée / 100 ms sortie | `ease-out` |
| Dialog, sheet | 200–250 ms entrée / 150 ms sortie | `--ease-out-quint` |
| Transition de page / état | 200–300 ms | `ease-in-out` |
| Progression (anneau/barre) | 500–800 ms | `ease-out` (c'est une *donnée*, on la laisse respirer) |
| Liste qui apparaît | 150 ms + stagger 30 ms, **max 8 éléments** staggerés | `ease-out` |

Règles : sortie plus rapide que l'entrée · n'animer que `transform` et `opacity` (et `stroke-dashoffset`) · jamais `width/height/top/left` · rien > 400 ms dans un flux de travail · une animation doit signifier quelque chose (cause → effet).

### 3.2 Outils, par ordre de préférence

1. **CSS + tw-animate-css** (déjà installé) : `animate-in fade-in-0 slide-in-from-bottom-2 duration-200`. shadcn l'utilise déjà pour Dialog/Popover. Couvre 90 % des besoins.
2. **View Transitions API** pour navigation et changements d'état : `import { ViewTransition } from "react"` + `experimental: { viewTransition: true }` dans `next.config.ts`. ⚠️ La doc Next 16 indique encore *expérimental, non recommandé en production* : l'activer seulement sur des transitions décoratives (fondu de contenu, morph carte → détail), jamais pour une info essentielle. Sans support navigateur, ça fonctionne simplement sans animation. Les animations ne se déclenchent que via Transitions/Suspense/`useDeferredValue` (une navigation Next en est une).
   ```tsx
   // layout du dashboard : fondu du contenu à chaque navigation
   <ViewTransition default="fade"><main id="main">{children}</main></ViewTransition>
   ```
   ```css
   ::view-transition-old(root), ::view-transition-new(root) { animation-duration: 180ms; }
   ```
3. **`motion`** (ex-framer-motion, `motion/react`) **uniquement** si besoin de spring/layout animations/AnimatePresence complexe (ex. réordonnancement de liste). Coût : `motion` complet ≈ 34 kB ; `LazyMotion` + `m` ≈ 4.6 kB au rendu initial + `domAnimation` (+15 kB) ou `domMax` (+25 kB, layout/drag). Pas installé aujourd'hui : un seul agent l'ajoute si un cas réel le justifie, et le note ici.
   ```tsx
   import { LazyMotion, domAnimation, MotionConfig } from "motion/react";
   <MotionConfig reducedMotion="user"><LazyMotion features={domAnimation} strict>{children}</LazyMotion></MotionConfig>
   // spring "UI" par défaut : { type: "spring", stiffness: 400, damping: 30 } (rapide, sans rebond excessif)
   ```

```tsx
// Compteur animé sans lib : CSS @property
// globals.css
@property --n { syntax: "<integer>"; initial-value: 0; inherits: false; }
.count { transition: --n 800ms ease-out; counter-reset: n var(--n); }
.count::after { content: counter(n); }
// Préférer toutefois afficher la valeur finale en texte pour les lecteurs d'écran.
```

### 3.3 `prefers-reduced-motion` (obligatoire)

- Tailwind : `motion-safe:animate-…` pour les animations décoratives, `motion-reduce:transition-none` / `motion-reduce:animate-none` sinon.
- Global de sécurité dans `globals.css` :
```css
@media (prefers-reduced-motion: reduce) {
  *, ::before, ::after { animation-duration: .01ms !important; animation-iteration-count: 1 !important; transition-duration: .01ms !important; scroll-behavior: auto !important; }
  ::view-transition-group(*), ::view-transition-old(*), ::view-transition-new(*) { animation: none !important; }
}
```
- Réduit ≠ supprimé : garder les changements d'opacité instantanés et le feedback d'état (check, couleur).
- `motion` : `<MotionConfig reducedMotion="user">` à la racine.

---

## 4. Accessibilité — WCAG 2.2 AA

### 4.1 Contrastes
- Texte normal ≥ 4.5:1, gros texte (≥ 24 px ou 18.66 px gras) ≥ 3:1, composants UI/icônes informatives/focus ≥ 3:1 (1.4.11).
- **Mesures réelles des tokens actuels** (`globals.css`, calcul OKLCH→sRGB, ratio WCAG) :

| Token | Light sur `background` blanc | Dark sur `background` / `card` | Verdict |
|---|---|---|---|
| `--brand` | 5.09 | 8.55 / 7.74 | OK texte |
| `--success` | **4.52** | 9.13 / 8.26 | Limite : OK sur blanc, **échoue sur `bg-muted`/fond teinté** → texte en `--success-foreground` |
| `--warning` | **3.77** ❌ | 10.4 / 9.41 | **Échoue en texte light** → ajouter `--warning-foreground: oklch(0.47 0.12 60)` (7.07:1) |
| `--info` | 4.85 | 8.02 / 7.26 | OK |
| `--destructive` | 4.76 | 6.84 / 6.19 | OK |
| `--muted-foreground` | 4.73 | 7.63 / 6.91 | OK sur blanc, **vérifier sur `bg-muted`** |
| `--ring` | **2.59** ❌ (et `outline-ring/50` encore moins) | 4.18 / 3.79 | **Focus light échoue 1.4.11** → `--ring: var(--brand)` en light (5.09) |
| `brand-foreground` sur `brand` | 4.87 | 8.08 | OK pour boutons |

- Jamais la couleur seule : statut = icône + texte (« 3 erreurs », pas un point rouge).

### 4.2 Clavier & focus
- Tout est atteignable au Tab, ordre logique, pas de piège (sauf dialog : focus trap + Échap + retour du focus au déclencheur — Radix le fait, ne pas le casser).
- Focus visible partout : `focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 ring-offset-background`. Ne jamais `outline-none` sans remplacement.
- **2.4.11 Focus Not Obscured** : la sidebar/header sticky ne doit pas cacher l'élément focus → `scroll-padding-top` = hauteur du header.
- Lien « Aller au contenu » en premier élément focusable.
- **2.5.7 Dragging** : tout glisser-déposer (réordonner langues/clés) a une alternative clic/clavier. **3.2.6 Consistent Help** : lien Aide/Docs au même endroit sur toutes les pages.
- Raccourcis : ⌘K/Ctrl+K palette ; pas de raccourcis à une seule touche sans possibilité de les désactiver (2.1.4).

### 4.3 États async → ARIA live
- Jobs de traduction (`job-row.tsx`) : région `role="status"` (polite) qui annonce début/fin (« Traduction FR terminée : 142 clés »), **pas chaque tick de %**. Erreurs : `role="alert"`.
- Barre de progression : `<Progress>` Radix expose `aria-valuenow` ; donner un `aria-label`.
- sonner : les toasts sont annoncés ; ne pas y mettre d'info critique uniquement (elle doit aussi rester dans la page).
- Boutons en chargement : `aria-busy`/`disabled` + texte « Traduction… » (pas seulement un spinner).

### 4.4 Formulaires
- `<Label htmlFor>` visible toujours (placeholder ≠ label). Description sous le champ via `aria-describedby`.
- Erreur : `aria-invalid="true"` + message lié par `aria-describedby`, texte = cause + correction. Focus sur le 1er champ en erreur à la soumission.
- **3.3.7 Redundant Entry** : ne pas redemander une info déjà saisie. **3.3.8 Accessible Authentication** : autoriser coller/gestionnaire de mots de passe, OAuth, pas de captcha cognitif.

### 4.5 Cibles
- **2.5.8** : ≥ 24×24 px minimum ; viser 36–40 px pour les contrôles desktop, 44 px tactile. Boutons icône shadcn `size="icon"` = 36 px OK ; ne pas descendre en dessous de `size-6` sans espacement.

### 4.6 RTL (ar)
- `<html dir={locale === "ar" ? "rtl" : "ltr"} lang={locale}>`.
- Propriétés **logiques** uniquement : `ms-*/me-*`, `ps-*/pe-*`, `start-*/end-*`, `text-start`, `border-s`, `rounded-s-*`. Interdit : `ml-`, `mr-`, `pl-`, `pr-`, `left-`, `right-`, `text-left` (sauf justification).
- Icônes directionnelles (chevrons, flèches) : `rtl:-scale-x-100`. Ne pas miroir les logos, les icônes de lecture vidéo, le code.
- Code, clés i18n, tokens : `dir="ltr"` sur le bloc (`<code dir="ltr">`).
- Tester chaque écran en `ar`.

### 4.7 Lecteurs d'écran
- Landmarks : `header`, `nav aria-label`, `main id="main"`, `aside`. Un seul `h1` par page, hiérarchie sans saut.
- Icônes décoratives `aria-hidden`, boutons icône avec `aria-label` (traduit).
- Changement de route : le titre (`<title>` via metadata) change et le focus va au `h1` ou `main`.
- Tester : VoiceOver (macOS/Safari) + NVDA (Firefox) sur le flux onboarding.

### 4.8 Outillage CI
```bash
pnpm add -D @playwright/test @axe-core/playwright
```
```ts
// web/e2e/a11y.spec.ts
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
for (const path of ["/en", "/en/login", "/en/signup", "/ar/app"]) {
  for (const scheme of ["light", "dark"] as const) {
    test(`a11y ${path} ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
      await page.goto(path);
      const { violations } = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
      expect(violations).toEqual([]);
    });
  }
}
```
axe ne détecte qu'environ 30–50 % des problèmes : compléter par un test clavier manuel + zoom 200 %/reflow 320 px.

---

## 5. Clarté & copywriting

- **Boutons = verbe + objet** : « Traduire 142 clés », « Connecter GitHub », « Copier la commande ». Jamais « OK », « Soumettre », « Cliquez ici ».
- **Erreurs = quoi + pourquoi + que faire** : ❌ « Erreur 422 » → ✅ « Le placeholder `{count}` manque dans la traduction allemande. Ajoutez-le ou régénérez la clé. » Utiliser `lib/use-error-message.ts`, mapper chaque code API vers un message actionnable.
- **Descriptions sous les champs** non évidents (1 ligne, `text-sm text-muted-foreground`). Ex. « Glossaire — termes que glot ne doit jamais traduire (noms de produit, marques). »
- **Tooltips** : seulement pour compléter (icône seule, raccourci clavier). Jamais d'info essentielle uniquement en tooltip (inaccessible au tactile). Délai 300–500 ms.
- **Nombres concrets** plutôt qu'adjectifs : « 3 erreurs dans 2 langues » > « Quelques problèmes ».
- **Ton** : direct, calme, tutoiement/vouvoiement cohérent par langue (fr : vouvoiement), pas de jargon interne, pas de point d'exclamation sauf célébration.
- **Confirmations destructives** : nommer l'objet (« Supprimer le projet *acme-web* ? 2 340 traductions seront perdues. ») ; préférer **Annuler** dans un toast pour les actions réversibles.
- **Densité** : dashboard dense mais respirant (rangées 40–44 px, `text-sm`), marketing aéré. Une idée par carte.
- **Toutes les chaînes dans `web/messages/*.json`** (dogfooding : glot traduit glot). ICU pour pluriels : `{count, plural, one {# clé} other {# clés}}`.

---

## 6. Système visuel

### 6.1 Tokens (dans `web/app/globals.css`, déjà en place — étendre, ne pas dupliquer)
- Couleurs sémantiques existantes : `background, foreground, card, muted, accent, primary, secondary, destructive, border, input, ring, brand (vert oklch 160), success, warning, info`.
- **À ajouter** : `--success-foreground`, `--warning-foreground`, `--info-foreground`, `--destructive-foreground` (texte lisible sur fonds teintés) + variantes `-subtle` pour fonds (ex. `oklch(from var(--success) l c h / 12%)`).
- Composants : **jamais de couleur brute** (`text-green-600`, hex) → toujours `text-success`, `bg-brand` etc.
- `--ring` light actuel = 2.59:1 → **à corriger** : `--ring: var(--brand)` (5.09:1) et focus `ring-2` plein (pas `/50`).

### 6.2 Typo
- Geist Sans / Geist Mono (déjà). Échelle : `text-xs 12` (méta seulement), `text-sm 14` (UI dashboard), `text-base 16` (corps marketing/forms), `text-lg 18`, `text-xl 20`, `text-2xl 24` (titres de page), `text-4xl–6xl` (marketing hero).
- `tabular-nums` sur tous les chiffres qui changent (%, compteurs). `text-balance` sur les titres, `text-pretty` sur paragraphes.
- Japonais/arabe : vérifier fallback (`Noto Sans JP`, `Noto Sans Arabic` ou police système) et `leading` plus grand pour l'arabe.

### 6.3 Espacements, rayons, ombres
- Grille de 4 px (Tailwind par défaut). Dashboard : `gap-2/3/4`, sections `space-y-6/8`. Marketing : sections `py-24`.
- Rayon : `--radius: 0.5rem` (lg). Cartes `rounded-xl`, boutons/inputs `rounded-md`, chips `rounded-full`.
- Ombres sobres : `shadow-xs` cartes, `shadow-lg` popovers/dialogs. En dark : bordures plutôt qu'ombres.

### 6.4 Icônes & illustrations
- lucide-react uniquement, `size-4` (UI), `size-5` (nav), `size-8` (états vides), `strokeWidth` par défaut. Pas d'emoji comme icône.
- Illustrations d'états vides : simples, monochromes (`text-muted-foreground`), SVG inline. Pas d'images lourdes dans le dashboard.

### 6.5 États de composants (chaque composant interactif doit les avoir)
| État | Rendu |
|---|---|
| default / hover | hover = `bg-accent` ou opacité 90 %, 150 ms |
| active/pressed | `active:scale-[0.98]` (motion-safe) |
| focus-visible | ring 2 px |
| disabled | `opacity-50 cursor-not-allowed` + raison en tooltip/description si pas évidente |
| loading | spinner `Loader2 animate-spin` + libellé « …ing », largeur stable (pas de saut) |
| skeleton | `components/ui/skeleton.tsx`, même géométrie que le contenu final |
| empty | §1.3 |
| error | bordure `destructive`, icône + message actionnable + « Réessayer » |
| success | check `text-success` + texte, éphémère (toast) ou persistant (badge) |

### 6.6 Dark mode
- `next-themes` déjà là, `system` par défaut. Tester **chaque** écran dans les 2 thèmes. Ne pas inverser les illustrations bêtement ; pas de blanc pur sur noir pur (tokens actuels OK).

---

## 7. Rapidité perçue

- **Optimistic UI** pour éditions de clé, toggles, ajout de langue : TanStack Query `onMutate` → rollback `onError` + toast. React 19 `useOptimistic` pour les Server Actions.
- **Skeletons** au lieu de spinners pour les chargements > 300 ms ; rien pour < 300 ms (évite le flash). `loading.tsx` par segment de route.
- **Prefetch** : `<Link>` Next prefetch par défaut ; `queryClient.prefetchQuery` au survol des projets/langues.
- **Listes longues** : `@tanstack/react-virtual` (déjà installé) au-delà de ~200 clés.
- **Palette ⌘K** (cmdk + `components/ui/command.tsx`) : naviguer projets/langues/clés, actions (« Traduire en… », « Copier le token », « Changer de thème »). Afficher `⌘K` dans la topbar.
- **Pas de layout shift** : réserver l'espace (CLS < 0.1), largeurs stables pour chiffres (`tabular-nums`) et boutons en chargement.
- Budgets : INP < 200 ms, LCP < 2.5 s (marketing), réponse visuelle à un clic < 100 ms.

---

## 8. Checklist finale (à cocher avant de rendre)

**Flux & clarté**
- [ ] Une seule action primaire par écran, libellée verbe + objet.
- [ ] Chaque état vide explique quoi faire et propose l'action.
- [ ] Chaque erreur dit la cause et la correction ; bouton Réessayer si pertinent.
- [ ] Commandes copiables avec token/slug pré-remplis, bouton Copier avec feedback.
- [ ] Attente async : indicateur live + aide après 30 s, jamais d'écran figé.
- [ ] Aucune chaîne en dur : tout dans `messages/*.json` (6 locales), pluriels ICU.

**Gamification**
- [ ] Toute progression affichée reflète une donnée réelle (couverture, validation).
- [ ] Célébrations rares, non bloquantes, désactivables, sans culpabilisation.
- [ ] Aucun anti-pattern §2.3 (streak quotidienne, FOMO, classements, emoji-icônes).

**Motion**
- [ ] Durées 100–300 ms (progression ≤ 800 ms), sortie plus rapide que l'entrée.
- [ ] Seulement `transform`/`opacity` animés.
- [ ] Testé avec `prefers-reduced-motion: reduce` : rien ne bouge inutilement, tout reste compréhensible.
- [ ] Pas de nouvelle dépendance d'animation sans cas réel (CSS/tw-animate-css d'abord).

**Accessibilité**
- [ ] Contraste ≥ 4.5:1 texte / ≥ 3:1 UI, vérifié en light ET dark.
- [ ] Parcours complet au clavier, focus toujours visible et non masqué.
- [ ] Dialogs : focus trap, Échap, retour du focus.
- [ ] `aria-live`/`role="status"` sur jobs et attentes ; `role="alert"` sur erreurs.
- [ ] Champs : label visible, description `aria-describedby`, `aria-invalid` + message.
- [ ] Boutons icône avec `aria-label` traduit ; icônes décoratives `aria-hidden`.
- [ ] Cibles ≥ 24 px (idéalement 36–44 px).
- [ ] Écran testé en `ar` (RTL) : propriétés logiques uniquement, icônes directionnelles miroir, code en `dir="ltr"`.
- [ ] Zoom 200 % et largeur 320 px sans perte ni scroll horizontal.
- [ ] axe-core (Playwright) : 0 violation wcag2a/2aa/21a/21aa/22aa, light + dark.

**Système & perf**
- [ ] Uniquement des tokens sémantiques (pas de hex/`text-green-600`).
- [ ] Composants shadcn existants réutilisés avant d'en créer.
- [ ] États hover/focus/active/disabled/loading/empty/error/success présents.
- [ ] Skeletons à la géométrie du contenu, CLS < 0.1, chiffres `tabular-nums`.
- [ ] Mutations optimistes avec rollback ; listes > 200 lignes virtualisées.
- [ ] Action disponible dans ⌘K si c'est une action fréquente.

---

## Sources

Onboarding / benchmarks
- Vercel — import d'un repo, détection framework, déploiement (+ confettis dans les anciennes docs) : https://vercel.com/docs/getting-started-with-vercel/import · https://vercel.com/docs/getting-started-with-vercel/template
- Linear — teardown onboarding (~1 min, ⌘K enseigné d'emblée) : https://www.candu.ai/blog/linear-onboarding-teardown · https://supademo.com/user-flow-examples/linear · https://linear.app/changelog/2019-05-09-first-time-user-experience
- Resend — sandbox `onboarding@resend.dev`, critique « clé API avant valeur » : https://resend.com/ · https://supademo.com/user-flow-examples/resend · https://www.saasui.design/pattern/onboarding/resend
- Clerk — setup interactif à la création : https://clerk.com/docs/getting-started/quickstart/setup-clerk
- PostHog — empty state de setup tant qu'aucune donnée n'arrive (`ProductEmptyState`) : https://www.saasui.design/pattern/empty-state/posthog · https://posthog.com/docs/product-tours/start-here.md
- Badges de progression i18n (Crowdin) : https://store.crowdin.com/products/bds · Shields endpoint : https://shields.io/badges/endpoint-badge

Gamification
- Streak creep (Decision Lab) : https://thedecisionlab.com/insights/consumer-insights/streak-creep-the-perils-of-too-much-gamification
- Duolingo, le bon et le mauvais : https://uxdesign.cc/the-good-the-bad-and-the-ugly-of-duolingo-gamification-3a12f0e80dc7
- Gamification éthique : https://uxdesign.cc/beyond-streaks-and-nudges-ethical-gamification-ai-design-1c4f6e8ef2cc · https://uxmag.com/articles/gamification-or-manipulation-understanding-the-ethics-of-engagement-loops · https://blog.nextbee.com/2025/12/15/the-dark-side-of-gamification-and-how-to-avoid-it/
- SDT & gamification : https://webspace.science.uu.nl/~veltk101/publications/art/chi2022-sdtws.pdf

Motion
- Next.js View Transitions : https://nextjs.org/docs/app/guides/view-transitions · https://nextjs.org/docs/app/api-reference/config/next-config-js/viewTransition
- Motion bundle size / LazyMotion : https://motion.dev/docs/react-reduce-bundle-size · MotionConfig : https://motion.dev/docs/react-motion-config
- canvas-confetti `disableForReducedMotion` : https://github.com/catdad/canvas-confetti
- Principes de durée/easing (Emil Kowalski, via skill) : https://emilkowal.ski/ui/great-animations

Accessibilité
- WCAG 2.2 (W3C) : https://www.w3.org/TR/WCAG22/ · nouveautés : https://www.w3.org/WAI/standards-guidelines/wcag/new-in-22/ · https://dequeuniversity.com/resources/wcag-2.2
- axe + Playwright : https://playwright.dev/docs/accessibility-testing · https://github.com/dequelabs/axe-core/blob/develop/doc/API.md#axe-core-tags
- ARIA live regions : https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Guides/Live_regions
- Tailwind `rtl:`/`ltr:` et propriétés logiques : https://tailwindcss.com/docs/hover-focus-and-other-states#rtl-support

Copywriting
- NN/g — messages d'erreur : https://www.nngroup.com/articles/error-message-guidelines/ · https://www.nngroup.com/videos/error-message-communication-guidelines/

Note : certaines sources benchmark sont des teardowns tiers (Supademo, Candu, saasui) et non des docs officielles ; les patterns restent valides mais les détails d'UI exacts peuvent avoir changé.
