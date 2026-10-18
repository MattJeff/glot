# glot knowledge snippet for AI app builders

Paste the block below into your builder's project instructions: Lovable (Project settings → Knowledge), Bolt (Project prompt / `.bolt/prompt`), v0 (Project instructions), Replit Agent (`replit.md`), or any `AGENTS.md`. It makes every generated screen translation-ready, and lets glot translate it later.

```text
Internationalization rules (this app will be translated with glot, https://github.com/MattJeff/glot):
- Never hardcode user-facing text in components. Every visible string (headings, buttons, labels, placeholders, aria-labels, titles, toasts, error and empty states, emails) goes through the i18n library:
  - Next.js App Router: next-intl. `const t = useTranslations("<section>")` (client) or `await getTranslations("<section>")` (server). Catalogs: messages/<locale>.json.
  - Vite / React: react-i18next. `const { t } = useTranslation()`, initialized in src/i18n.ts and imported first in src/main.tsx. Catalogs: src/locales/<locale>.json.
- Add new strings to the English catalog only, with nested keys named by screen and role, e.g. "billing.plan_card.upgrade_button". Do not create or edit other locales' files; glot generates them.
- Use placeholders and ICU plurals, never concatenation: t("invoices.count", { count }) with "{count, plural, one {# invoice} other {# invoices}}" (next-intl) or "{{count}} invoices" with _one/_other keys (i18next).
- Format dates, numbers and currencies with Intl (or the library's formatters) using the active locale; never build them by hand.
- Leave room for text that is 30-40% longer than English: no fixed widths on buttons, tabs or badges; allow wrapping.
- Brand and product names stay untranslated; keep them in the copy as-is.
- When asked to "translate the app" or "add a language": run `npx -y glot-i18n init --yes --targets <locales>`, then `npx -y glot-i18n i18nize --yes` for any remaining hardcoded strings, then `npx -y glot-i18n translate --yes` (needs ANTHROPIC_API_KEY) and `npx -y glot-i18n check`. If you cannot run commands, say that the repository's "glot onboard" GitHub workflow will do it.
```
