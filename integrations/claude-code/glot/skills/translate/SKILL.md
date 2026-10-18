---
name: translate
description: Translate missing and stale strings with glot, validate them and fix layout overflow.
argument-hint: "[locales, e.g. fr,de]"
disable-model-invocation: true
---

Translate this app's catalogs with glot (use `glot`, or `npx -y glot-i18n` if it is not on PATH). Locales: $ARGUMENTS (empty = all targets in glot.toml).

1. If there is no `glot.toml`, run `/glot:i18nize` first.
2. `glot translate --dry-run` (add `--locale $ARGUMENTS` when given) and show the plan and estimated cost.
3. If `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` is not set, stop and tell the user which variable glot.toml expects.
4. `glot translate --yes` (same `--locale`).
5. `glot check`; fix every error (`glot set <locale> <key> "<value>"` or `glot translate --key <key> --force`) until it exits 0.
6. If a dev server is running, `glot fit --base-url <its URL> --routes <main routes>` to shorten overflowing translations.
7. Summarize coverage with `glot status`.
