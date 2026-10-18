---
name: i18nize
description: Convert hardcoded UI strings in this app (or in the given paths) to translation keys with glot.
argument-hint: "[paths...]"
disable-model-invocation: true
---

Internationalize this app with glot (use `glot`, or `npx -y glot-i18n` if it is not on PATH). Paths to convert: $ARGUMENTS (empty = the whole project).

1. If there is no `glot.toml`: install the i18n library if missing (next-intl for Next.js App Router, react-i18next + i18next otherwise), then `glot init --yes` (ask which target languages if unknown).
2. `glot extract --json` and summarize what will be converted.
3. `glot i18nize --dry-run $ARGUMENTS`, check the diff for anything that is not user-facing text (CSS classes, URLs, enum values, log messages) and exclude it by passing narrower paths.
4. `glot i18nize --yes $ARGUMENTS`.
5. Wire the i18n library once if it is not wired yet (provider/init file loading the catalogs listed in glot.toml), then make sure the app still builds or typechecks.
6. Report the number of files and keys, and suggest `/glot:translate`.
