import type { ReactNode } from "react";
import { NextIntlClientProvider } from "next-intl";
import { getLocale, getMessages } from "next-intl/server";
import { GlotFallbackProvider } from "@glot/runtime/next-intl";
import glossary from "../glossary.json";

export const metadata = { title: "glot runtime + Next", icons: "data:," };

export default async function Layout({ children }: { children: ReactNode }) {
  // A locale served by glot on demand counts as shipped: no on-device pass over an already translated page.
  const locale = await getLocale();
  return (
    <html lang={locale}>
      <body>
        <NextIntlClientProvider>
          <GlotFallbackProvider shipped={["en", locale]} source={await getMessages()} glossary={glossary} wasm="/glot.wasm">
            {children}
          </GlotFallbackProvider>
        </NextIntlClientProvider>
      </body>
    </html>
  );
}
