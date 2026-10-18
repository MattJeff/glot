# Launch checklist (v0.2.1)

Drafts in this folder: [Show HN](show-hn.md), [Product Hunt](product-hunt.md), [X + LinkedIn](social.md), [Reddit](reddit.md).

## Before announcing

- [ ] Release `v0.2.1` published with all binaries; `npx glot-i18n@0.2.1 --version`, `brew install mattjeff/glot/glot`, `install.sh` and the Claude Code plugin all install 0.2.1 (per-channel steps: [`integrations/README.md`](../../integrations/README.md#release-checklist-per-glot-version)).
- [ ] MCP registry entry `io.github.MattJeff/glot` shows 0.2.1.
- [ ] `MattJeff/glot/action@v0` points at a commit that ships 0.2.1.
- [ ] CLI reference regenerated from the release binary: `cd web && pnpm gen:cli-docs` (or `node scripts/gen-cli-docs.ts path/to/glot`), commit `web/content/docs/cli.md`.
- [ ] Docs reviewed against the release: commands added since (SEO, emails/images, DB, feedback, coverage/variants, A/B, on-demand server translation) have a page or a section, or are left out of the posts.
- [ ] `/docs` deployed somewhere public, or the posts link to `web/content/docs/` on GitHub (the README already does).
- [ ] Every number in the drafts checked against a real run on the release:
  - [ ] 117 hardcoded strings → nothing to report (`examples/demo.sh`, verified on 0.2.1 with the mock provider);
  - [ ] **$0.035 for 850 strings with `--batch`**: write down the setup (model, locales, glossary size, date) in the README or docs before quoting it; rerun if the prompt changed since;
  - [ ] "-50 %" batch and "~10 %" cached prefix match Anthropic's current pricing;
  - [ ] overlay "~6 kB gzip" (`packages/overlay`, `pnpm build`).
- [x] fit before/after re-recorded with a real model ("Zahlungserinnerung senden" → "Mahnung", claude-haiku-5-5, $0.0002).
- [ ] Re-record the refusal demo with a real model doing the retries (step 6), and update its caption in the README and the landing (`landing.hero.demoCaption`).
- [ ] Re-record i18nize / overlay / locale matrix with a real model (they use the mock provider, which prefixes `⟦de⟧`), see below.
- [ ] README renders on GitHub: GIFs load (i18nize.gif is ~5.5 MB), links resolve.
- [x] Locale matrix: the source-locale frame now keeps its `/{locale}` prefix, so cookie routing no longer leaks another frame's locale.
- [ ] Issue templates and a `good first issue` label exist; Discussions enabled.
- [ ] Someone else ran the quickstart from a clean machine.

## Launch day

- [ ] Show HN Tuesday–Thursday, 8–10am ET. Stay in the thread 3 hours. No vote requests anywhere.
- [ ] Product Hunt 00:01 PT the same or next day (secondary).
- [ ] X thread and LinkedIn post after HN is live, linking the repo (not HN).
- [ ] Reddit: r/nextjs one day, r/webdev another, after reading each sub's rules.
- [ ] Log every bug report as an issue within the day; reply with the issue link.

## After

- [ ] Write down what questions came up twice and add them to `web/content/docs/faq.md`.
- [ ] Patch release for launch-day bugs.

## Re-recording the demos

All recordings are real runs driven by Playwright (`packages/visual-qa` must have `pnpm install` + `pnpm exec playwright install chromium`); videos are WebM, converted to GIF with ffmpeg. Scripts: [`record/`](record). `WORK` is any scratch directory.

```sh
# 1. The demo app, translated and running (add GLOT_REAL=1 ANTHROPIC_API_KEY=... for real translations)
KEEP=1 PORT=3255 examples/demo.sh > $WORK/demo.log      # prints the kept copy path at the end
cd <kept copy> && pnpm exec next dev -p 3255 &

# 2. Terminal: glot extract → i18nize → extract
cp -r examples/demo-saas $WORK/saas && cd $WORK/saas && rm -rf node_modules && glot init --yes --targets fr,de,ja --provider mock
glot i18nize --dry-run app/page.tsx > $WORK/i18nize.txt
python3 docs/launch/record/build-term.py $WORK && node docs/launch/record/term.mjs $WORK

# 3. fit before/after: set a long German label by hand, screenshot, fit, screenshot
glot set de invoice_actions.send_reminder_button "Zahlungserinnerung senden"
node docs/launch/record/fit-shot.mjs $WORK fit-before
glot fit --yes --base-url http://localhost:3255 --routes /invoices/inv_1042 --locales de
node docs/launch/record/fit-shot.mjs $WORK fit-after && node docs/launch/record/fit-compose.mjs $WORK

# 4. Overlay: add <script src="http://127.0.0.1:4317/overlay.js" defer /> to the copy's app/layout.tsx <head>
glot dev --token launchdemo &
node docs/launch/record/overlay.mjs $WORK

# 5. Locale matrix: dashboard on :3177 (next start) + glot-server on :8790, a user, a project `acme-invoices`
#    with preview settings {base_url: "http://localhost:3255", url_pattern: "/{locale}{route}"}; session cookie in $WORK/cj
node docs/launch/record/matrix.mjs $WORK 1

# 6. Refusal (README hero): real glot binary, LLM scripted to fail on the first pass
CARGO_TARGET_DIR=target/integ cargo build -p glot-cli
docs/launch/record/refusal.sh $WORK/refusal
#    with a real model doing the retries (first pass still injected; any OpenAI-compatible endpoint):
UPSTREAM=https://api.anthropic.com/v1 UPSTREAM_KEY=$ANTHROPIC_API_KEY MODEL=claude-haiku-5-5 docs/launch/record/refusal.sh $WORK/refusal
LABEL="first pass injected, retries by claude-haiku-5-5" python3 docs/launch/record/refusal-term.py $WORK/refusal
node docs/launch/record/term.mjs $WORK/refusal     # then ffmpeg below -> web/public/launch/refusal.{webm,gif}, last frame -> refusal.png

# GIFs
ffmpeg -i in.webm -vf "fps=10,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" out.gif
```

Outputs go to `web/public/launch/` (`refusal`, `i18nize`, `overlay`, `locale-matrix` as `.gif` + `.webm`, `fit-before-after.png`, `overlay.png`, `locale-matrix.png`). Scripts write their files under `$WORK` (videos in `$WORK/vid/…`).
