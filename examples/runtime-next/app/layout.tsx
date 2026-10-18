import type { ReactNode } from "react";
import { NextIntlClientProvider } from "next-intl";
import { getMessages } from "next-intl/server";
import { GlotFallbackProvider } from "@glot/runtime/next-intl";
import glossary from "../glossary.json";

export const metadata = { title: "glot runtime + Next", icons: "data:," };

export default async function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <NextIntlClientProvider>
          <GlotFallbackProvider shipped={["en"]} source={await getMessages()} glossary={glossary} wasm="/glot.wasm">
            {children}
          </GlotFallbackProvider>
        </NextIntlClientProvider>
      </body>
    </html>
  );
}
