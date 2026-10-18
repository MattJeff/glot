# glot commands

| Command | What it does |
|---|---|
| `glot localize [--targets fr,de] [--dry-run] [--yes] [--provider mock]` | init if needed → i18nize → translate → check → status. Idempotent. |
| `glot init --yes --targets fr,de` | Detect the stack; write glot.toml, glossary.toml, .mcp.json, agent rules, `.agents/skills/glot`, CI workflow. |
| `glot status [--json]` | Coverage per locale and the translation plan. |
| `glot extract --json` | Hardcoded strings, missing and unused keys. |
| `glot i18nize [paths] [--dry-run] [--yes]` | Rewrite hardcoded copy to `t("key")` and wire the i18n library. |
| `glot translate [--locale fr] [--key k] [--force] [--dry-run] [--yes] [--batch]` | Translate missing/stale keys (validated, glossary enforced). `--batch`: Anthropic Message Batches, -50 %. |
| `glot check [--format text\|json\|sarif\|github]` | Placeholders, plurals, glossary, length. Exit 1 on errors. |
| `glot add <key> "<text>" --note "<context>"` | Add a source string. |
| `glot set <locale> <key> "<value>"` | Deliberate human translation (validated, recorded as reviewed). |
| `glot glossary add <Term> --dnt \| --fr "<t>"` | Glossary term. |
| `glot fit --base-url <url> --routes /,/x` | Shorten translations overflowing in the running app. |
| `glot provenance export --format json\|csv\|html` | Per key/locale: machine (model, date) / human / reviewed (by whom, when), score. |
| `glot provenance catalog` | Write `messages/{locale}.provenance.json` for the runtime badge (`@glot/runtime/provenance`). |
| `glot agent-prompt [--for lovable\|bolt\|v0\|agent]` | Print a ready-to-paste i18n prompt for the detected stack. |
| `glot mcp` | MCP server on stdio (`propose_string`, `add_string`, `translate`, `check`, …). |

Config lives in `glot.toml` (`[llm] provider/model/api_key_env/budget_usd`, `[qa]`, `[extract]`). Set `GLOT_REVIEWER="Name"` so `glot set` records who reviewed.
