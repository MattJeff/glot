import type { ReactNode } from "react";
import Link from "next/link";
import { NextIntlClientProvider } from "next-intl";
import { getLocale, getTranslations } from "next-intl/server";
import { headers } from "next/headers";
import { GlotExperiments } from "@glot/runtime/next-intl";
import { Beacon } from "./beacon";
import { href } from "./href";
import "./globals.css";

export const metadata = { title: "Relay: invoicing that gets you paid", icons: "data:," };


export default async function Layout({ children }: { children: ReactNode }) {
  const locale = await getLocale();
  const t = await getTranslations();
  return (
    <html lang={locale}>
      <body>
        <NextIntlClientProvider>
          <GlotExperiments header={(await headers()).get("x-glot-xp")}>
          <header className="nav">
            <Link href={href(locale, "/")} className="logo">Relay</Link>
            <nav>
              <Link href={href(locale, "/")}>{t("nav.home")}</Link>
              <Link href={href(locale, "/pricing")}>{t("nav.pricing")}</Link>
              <Link href={href(locale, "/signup")} className="btn" id="nav-cta">{t("home.hero.cta")}</Link>
            </nav>
          </header>
          {children}
          <footer>{t("footer")}</footer>
          <Beacon locale={locale} glotKey={process.env.GLOT_KEY} endpoint={process.env.GLOT_SERVER} />
          </GlotExperiments>
        </NextIntlClientProvider>
      </body>
    </html>
  );
}
