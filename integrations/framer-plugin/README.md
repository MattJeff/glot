# glot for Framer

Translates a Framer project's localization sources (pages, CMS, components) with glot and writes the translations back. Full guide: [docs/integrations#framer](../../web/content/docs/integrations.md#framer).

```sh
pnpm install
pnpm dev                       # plugin: Framer → Plugins → Open Development Plugin (https://localhost:5173)
pnpm build && pnpm run pack    # plugin.zip, to submit to the Marketplace or share
FRAMER_API_KEY=… GLOT_TOKEN=glot_… pnpm sync <framer project URL> <glot project slug> [--no-translate] [--force] [--publish]
pnpm test                      # sync logic against a Framer double and a simulated glot server
```

`src/sync.ts` holds the logic for both entry points: `src/main.ts` (plugin, Localization mode) and `src/cli.ts` (Server API, `framer-api`, Node 22+).
