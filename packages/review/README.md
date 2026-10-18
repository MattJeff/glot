# @glot-i18n/review

Live review of translations **on your real site**: reviewers click a text, comment, propose a new wording,
ask the AI to reword it, edit every language by hand, and see each other's notes appear in real time.
Normal visitors download a 0.5 kB loader and nothing else runs.

```html
<script src="https://unpkg.com/@glot-i18n/review/dist/review.global.js" data-project="acme"
        data-endpoint="https://glot.example.com" data-app-url="https://app.glot.dev" async></script>
```

```ts
import { glotReview } from "@glot-i18n/review";
glotReview.init({ project: "acme", endpoint: "https://glot.example.com", appUrl: "https://app.glot.dev" });
```

Review mode turns on only with a review token: a review link (`https://your.site/page?glot_review=glr_…`,
created in glot → Review → Links), or a member session minted by the glot Review page. The token is moved to
`sessionStorage` and removed from the address bar. Declare your site origin in glot → Review → Settings (CORS).

| Option (`data-*`) | |
|---|---|
| `project` | project slug |
| `endpoint` | glot server origin (default: same origin) |
| `locale` | locale shown (default `<html lang>`) |
| `appUrl` (`app-url`) | glot web app, for “Open in glot” |
| `appOrigin` (`app-origin`) | origin of the glot Review page when the site runs in its iframe |
| `lang`, `dir`, `labels` | widget language / direction / strings (built in: English, French) |

What reviewers get:
- a floating, draggable toolbar (drag the grip or use arrow keys): Comment mode, annotation list, status filter,
  current locale, “Open in glot”, leave review;
- Comment mode (button or `Alt+Shift+R`): hover highlights a text and shows its key; `Tab`/`Shift+Tab` walk the
  texts, `Enter` opens, `Esc` quits. On phones: long-press any text;
- a popover per text with three tabs: **Discussion** (thread, reactions, proposal, ✨ reword: shorter / clearer /
  more formal / warmer / fit the segment, proposals validated in every language, in-page preview, apply or submit),
  **Text** (manual edit of the current language with placeholders highlighted and protected, live validator, save
  or submit for approval, reset, previous version, delete the translation; create a key from hard-coded text),
  **All languages** (every locale, saved atomically);
- numbered pins that follow scroll, resize and DOM changes, re-anchored by key, then selector, then text
  (“orphan” in the list when the text is gone); a cropped capture is attached to each new annotation.

Keys are detected from `[data-glot-key]` or the dev markers of `@glot-i18n/runtime/dev`; otherwise the server
matches the text. Shadow DOM, no global CSS, ~17 kB gzip loaded only for reviewers. Protocol:
`docs/design/live-review-contract.md`.

Tests: `pnpm build && pnpm test` (Playwright against `test/fake.ts`, a server following the contract;
`node test/fake.ts 3501` serves the demo at `http://127.0.0.1:3501/?glot_review=glr_edit`).
Against a real server: `GLOT_REAL=http://127.0.0.1:8841 node --test test/real.test.ts` (creates its own user + project).

MIT — part of [glot](https://github.com/MattJeff/glot).
