// Onboarding commands, pre-filled with the real server URL, slug and token. Code is not translated.
export type Ctx = { server: string; slug: string; token: string; targets: string[] };

const mcpUrl = (c: Ctx) => `${c.server}/mcp?project=${c.slug}`;

export const claudeCode = (c: Ctx) =>
  `claude mcp add --transport http glot "${mcpUrl(c)}" \\\n  --header "Authorization: Bearer ${c.token}"`;

export const mcpJson = (c: Ctx) =>
  JSON.stringify({ mcpServers: { glot: { type: "http", url: mcpUrl(c), headers: { Authorization: `Bearer ${c.token}` } } } }, null, 2);

export const agentPrompt = (c: Ctx) =>
  `Use the glot MCP server to find the user-facing strings in this app, add them to the project, and translate them into ${c.targets.join(", ") || "French"}.`;

export const localize = (c: Ctx) => `npx glot-i18n localize --yes --targets ${c.targets.join(",") || "fr"}`;

export const push = (c: Ctx) =>
  `cat >> glot.toml <<'EOF'\n\n[server]\nurl = "${c.server}"\nproject = "${c.slug}"\nEOF\nexport GLOT_TOKEN=${c.token}\nnpx glot-i18n push`;

export const badge = (c: Ctx) =>
  `[![i18n](https://img.shields.io/endpoint?url=${encodeURIComponent(`${c.server}/badge/${c.slug}.json`)})](https://glotlang.com)`;
