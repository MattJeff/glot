# @glot/visual-qa

Visual QA for localized UIs, driven by Playwright: renders every route × locale × viewport and reports
overflowing, clamped, overlapping, untranslated and tofu text (`report.json`, `report.md`, crops).
It is the engine behind `glot measure`, `glot fit`, `glot translate --visual` and `glot coverage --render`.

```sh
npm i -g @glot/visual-qa          # Node >= 20
npx playwright install chromium   # once
glot-visual-qa --base-url http://localhost:3000 --routes /,/pricing --locales fr,de --source-locale en
```

`glot` finds it in this order: `$GLOT_VISUAL_QA` (path to `dist/cli.js` or an executable), `glot-visual-qa`
on `PATH`, else `npx -y @glot/visual-qa@<glot major.minor>`.

`glot-visual-qa --help` lists every option (`--url-pattern`, `--viewports`, `--catalogs`, `--measure`,
`--harness`, `--fail-on`, `--upload`). Exit code 1 when an issue kind listed in `--fail-on` is found.

MIT — part of [glot](https://github.com/MattJeff/glot).
