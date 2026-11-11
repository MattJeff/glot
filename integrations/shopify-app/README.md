# glot for Shopify (app embed)

A custom app whose only extension is a theme app extension with one **app embed block** (`extensions/glot-embed/blocks/glot.liquid`): the glot script and a language switcher on every storefront page of an Online Store 2.0 theme. Full guide: [docs/integrations#shopify-app-embed](../../web/content/docs/integrations.md#shopify-app-embed).

```sh
pnpm test                    # renders the embed with liquidjs (Shopify tags stubbed) and checks the schema translations
shopify app config link      # links shopify.app.toml to your Dev Dashboard app (fills client_id)
shopify app deploy           # releases a version with the extension; then install the app on your store
```
