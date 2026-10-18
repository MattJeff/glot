# glot for AI app builders (Lovable, Bolt, v0, Replit)

AI builders ship English-only apps by default. Two moves fix that:

1. **Before generating**: paste the [knowledge snippet](ai-builders-prompt.md) into the project instructions, so every string goes through next-intl or react-i18next from the first prompt.
2. **After generating**: run glot on the code, either from a shell or with the one-click [`glot-onboard`](workflows/glot-onboard.yml) workflow that opens a pull request.

You need an LLM key (`ANTHROPIC_API_KEY`) only for translation; extraction, rewriting and checks are deterministic.

## Translate this app

[![Translate this app with glot](https://img.shields.io/badge/Translate_this_app-glot-2563eb?logo=googletranslate&logoColor=white)](https://github.com/MattJeff/glot/blob/main/integrations/ai-builders.md#translate-this-app)

The button points here. Its contract: one workflow file, one click, one pull request.

1. Add the workflow to your app's repository (needs `gh` with the `workflow` scope: `gh auth refresh -s workflow`):

   ```sh
   APP=you/your-app
   gh api -X PUT "repos/$APP/contents/.github/workflows/glot-onboard.yml" \
     -f message="Add glot onboard workflow" \
     -f content="$(curl -fsSL https://raw.githubusercontent.com/MattJeff/glot/main/integrations/workflows/glot-onboard.yml | base64 | tr -d '\n')"
   ```

   Or by hand: create `.github/workflows/glot-onboard.yml` with the content of [`workflows/glot-onboard.yml`](workflows/glot-onboard.yml).
2. Add the `ANTHROPIC_API_KEY` secret (Settings → Secrets and variables → Actions). Without it the PR contains the extracted strings but no translations.
3. Allow the workflow to open PRs: Settings → Actions → General → "Allow GitHub Actions to create and approve pull requests".
4. Click **Run workflow** at `https://github.com/<you>/<your-app>/actions/workflows/glot-onboard.yml`, or run `gh workflow run glot-onboard.yml -R "$APP" -f targets=fr,es,de`.

The workflow:

- installs `i18next` + `react-i18next` (Vite/React) or `next-intl` (Next.js) if no i18n library is present, with the repo's package manager;
- for Vite/React apps, writes `src/i18n.ts` and imports it from `src/main.tsx`; for Next.js, the PR lists the three next-intl wiring steps;
- runs `glot init`, `glot i18nize`, `glot translate` (if the key is set);
- opens a PR with per-locale coverage and the `glot check` result.

Optional secret `GLOT_TOKEN` (a PAT with `workflow` scope) lets the PR also add `.github/workflows/glot.yml`, which keeps translations up to date on every later PR. Without it, add that file yourself (`glot init` writes it, or copy the snippet from the main README).

## Per tool

### Lovable

1. Project settings → Knowledge: paste the [snippet](ai-builders-prompt.md).
2. Connect GitHub (Integrations → GitHub). Lovable syncs both ways with the repo's default branch.
3. In the GitHub repo, follow [Translate this app](#translate-this-app). Merge the PR: Lovable pulls the change back into the editor.
4. Lovable commits straight to `main`, so PR-triggered workflows do not see its edits. Re-run **glot onboard** whenever you want to catch up: it skips `init`, converts newly hardcoded strings, translates the delta and opens a new PR.

### Bolt.new

1. Add the snippet to the project prompt.
2. Export to GitHub (or download the project and push it to a new repo).
3. Follow [Translate this app](#translate-this-app). To keep iterating in Bolt, re-import the repo after merging.

### v0

1. Add the snippet to the project instructions. v0 generates Next.js, so the workflow installs `next-intl`; finish its 3-step wiring as the PR describes (or ask v0 to do it with the PR's checklist).
2. Push the project to GitHub (v0's GitHub integration), then follow [Translate this app](#translate-this-app).

### Replit

Replit has a shell, so run glot directly:

```sh
npx -y glot-i18n init --yes --targets fr,es,de
npx -y glot-i18n i18nize --yes
export ANTHROPIC_API_KEY=...        # or add it in Secrets
npx -y glot-i18n translate --yes
npx -y glot-i18n check
```

Put the snippet in `replit.md` so Replit Agent keeps strings in the catalogs afterwards. Wire the library once if `i18nize` reports it (`src/i18n.ts` for react-i18next, `NextIntlClientProvider` for next-intl).

### Cursor, Claude Code, Codex, Windsurf

See [README.md](README.md#coding-agents).
