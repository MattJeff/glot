import { useTranslations } from "next-intl";

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

export function useDemo() {
  const t = useTranslations("lib.snippets");
  return {
  toolCall: t("demo.translate_call"),
  source: t("plurals.checkout_items"),
  diff: [
    { sign: "+", code: t("plurals.items") },
    { sign: "+", code: t("actions.pay_button") },
    { sign: "~", code: t("status.lock_entries") },
  ],
  file: "messages/fr.json",
};
}
