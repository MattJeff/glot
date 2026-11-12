# glot for Shopify (app embed)

A custom app whose only extension is a theme app extension with one **app embed block** (`extensions/glot-embed/blocks/glot.liquid`): the glot script and a language switcher on every storefront page of an Online Store 2.0 theme. Full guide: [docs/integrations#shopify-app-embed](../../web/content/docs/integrations.md#shopify-app-embed).

```sh
pnpm test                    # renders the embed with liquidjs (Shopify tags stubbed) and checks the schema translations
shopify app config link      # links shopify.app.toml to your Dev Dashboard app (fills client_id)
shopify app deploy           # releases a version with the extension; then install the app on your store
```

## Two configurations

- `shopify.app.toml`: a merchant's own custom app (the embed only; add the connector's scopes there to reuse the app's Client ID and secret in glot's Shopify connector form).
- `shopify.app.glot.toml` (`--config glot`): glot's own app, for whoever operates a glot server. It adds the connector's scopes, the OAuth redirect URL (`{GLOT_WEB_ORIGIN}/api/connectors/oauth/shopify/callback`) and the app webhooks (`app/uninstalled` and the three mandatory compliance topics, all to `{GLOT_PUBLIC_URL}/api/connectors/shopify/webhooks`). With `GLOT_SHOPIFY_CLIENT_ID` / `GLOT_SHOPIFY_CLIENT_SECRET` set on the server, the dashboard's Shopify card shows a one-click **Connect**.
