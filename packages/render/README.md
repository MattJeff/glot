# @glot/render

Client of the glot render API: hand it a piece of content and a recipient's locale, get the content back in that language.

```ts
import { createClient } from "@glot/render";

const glot = createClient({ key: process.env.GLOT_RENDER_KEY!, baseUrl: "https://api.glotlang.com" });

const mail = await glot.render({
  locale: user.locale,
  type: "email",
  content: { subject: "Your invoice {{number}}", html: invoiceHtml },
  variables: { number: "F-2026-0042", total: { type: "currency", value: 49, currency: "EUR" } },
});
await resend.emails.send({ to: user.email, subject: mail.content.subject, html: mail.content.html });
```

Server-side only (the key is a secret). No dependencies. When glot cannot be reached, `render` returns the source
content with variables filled and `status: "fallback"` instead of throwing. Full reference: `web/content/docs/render.md`.
