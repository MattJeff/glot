// Code samples shown in the UI. Code is not translated, so it lives outside the message catalogs.

// ponytail: placeholder repo URL, update when the GitHub org is final.
export const GITHUB_URL = "https://github.com/MattJeff/glot";

export const INSTALL_CARGO = "cargo install glot";
export const INSTALL_NPX = "npx glot init";

export function mcpJson(token = "<your-token>", url = "http://localhost:8790") {
  return JSON.stringify(
    { mcpServers: { glot: { command: "glot", args: ["mcp"], env: { GLOT_SERVER: url, GLOT_TOKEN: token } } } },
    null,
    2,
  );
}

export function cliLogin(url = "http://localhost:8790") {
  return `glot login --server ${url}`;
}

export const DEMO = {
  toolCall: `glot.translate({
  keys: ["checkout.items"],
  locales: ["fr", "ja", "ar"]
})`,
  source: `"checkout.items": "{count, plural,
  one {# item} other {# items}}"`,
  diff: [
    { sign: "+", code: `"items": "{count, plural, one {# article} other {# articles}}"` },
    { sign: "+", code: `"pay": "Payer {amount}"` },
    { sign: "~", code: "glot.lock  2 entries" },
  ],
  file: "messages/fr.json",
};

/** The universal website tag (glot.js, packages/edge/src/website.global.ts): what the platform guides tell users to paste. */
export const UNIVERSAL_TAG = `<script src="https://app.glotlang.com/widget/glot.js" data-project="your-project" data-key="pk_…" async></script>`;

/** glot Edge subdomain records (docs/edge.md → Subdomain). */
export const EDGE_DNS = `es.example.com.        CNAME  edge.glotlang.com.
_glot.es.example.com.  TXT    "<token shown in the dashboard>"`;
