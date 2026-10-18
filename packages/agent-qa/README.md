# @glot/agent-qa

Task parity QA: the same tasks (`glot.tasks.yaml`) run in every locale of your app, by a Claude vision
agent or a scripted mock, and each failure is blamed on a translation key with a fix. Engine behind
`glot parity`.

```sh
npm i -g @glot/agent-qa           # Node >= 20
npx playwright install chromium   # once
glot-agent-qa --tasks glot.tasks.yaml --base-url http://localhost:3000 --locales en,fr,de \
  --catalogs "messages/{locale}.json" --provider mock
```

```yaml
# glot.tasks.yaml
- id: pricing
  goal: Open the pricing page
  start: /
  success: { url_matches: "^/pricing" }
  script: [{ click: nav.pricing }]   # --provider mock: click the control labelled with nav.pricing's translation
```

- Client-side navigations (Next `<Link>`, `router.push`) are awaited: URL change, network idle, then the
  success condition is polled for up to 5 s.
- `--locale-cookie` (default `NEXT_LOCALE,glot-locale`, the cookies `glot i18nize` generates) is set to
  the locale before each run so in-app links keep the language; `--locale-cookie ""` disables it.
- `--provider anthropic` needs `ANTHROPIC_API_KEY`. Exit 1 when a locale is below `--min-parity`.

`glot` finds it in this order: `$GLOT_AGENT_QA`, `glot-agent-qa` on `PATH`, else
`npx -y @glot/agent-qa@<glot major.minor>`.

MIT — part of [glot](https://github.com/MattJeff/glot).
