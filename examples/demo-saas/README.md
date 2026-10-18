# Acme Invoices — glot demo app

A small but realistic Next.js SaaS (App Router, TypeScript, Tailwind v4) that is **deliberately not internationalized**: every string is hardcoded English in JSX, with no i18n library and no message files. It is the playground for `glot` demos (`extract`, `i18nize`, `translate`, `check`, `fit`, visual QA).

```sh
pnpm install
pnpm dev        # http://localhost:3200
pnpm build
```

Routes: `/` (dashboard), `/invoices`, `/invoices/inv_1042`, `/settings/team`, `/settings/billing`, `/pricing`, `/onboarding`.

## Traps on purpose

| Trap | Where |
|---|---|
| Plurals by concatenation (`"You have " + n + " overdue invoices"`, `pluralize()`) | `app/page.tsx`, `components/InvoicesView.tsx`, `lib/format.ts` |
| Template literals with variables (`` `Invitation sent to ${email}` ``, `` `${remaining} steps left` ``) | `InviteMemberForm`, `OnboardingChecklist`, `lib/format.ts` |
| User-facing copy in config objects/arrays | `lib/data.ts` (`STATUS_META`, `NAV`, `PLANS`, `ONBOARDING_STEPS`), `FAQ`, `FILTERS`, `ACTIVITY` |
| Fixed-width buttons that overflow in German (`w-20`, `w-28`, `w-36`) | `InvoiceActions`, `NewInvoiceForm`, `EmptyState`, billing/team settings |
| Short ambiguous labels: "Save", "Close", "Archive" (verb vs. noun), "Remove" | `NewInvoiceForm`, `Toast`, `InvoiceActions`, team settings |
| Attributes: `placeholder`, `aria-label`, `title`, `<caption>`, `sr-only` | forms, `SearchBar`, `InvoiceTable` |
| Locale-bound formatting hardcoded to `en-US` | `lib/format.ts` |
| Metadata (`<title>`, description) | `app/layout.tsx`, `app/invoices/page.tsx` |

Never commit an i18nized version of this folder: `examples/demo.sh` works on a temporary copy.
