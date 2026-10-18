import type { ReactNode } from "react";
import Link from "next/link";
import { notFound } from "next/navigation";
import { LOCALES, isLocale, translator } from "../i18n";
import { Review } from "../review";
import "../globals.css";

export const metadata = { title: "Relay — glot live review demo", icons: "data:," };
export const generateStaticParams = () => LOCALES.map((locale) => ({ locale }));

export default async function Layout({ children, params }: { children: ReactNode; params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  if (!isLocale(locale)) notFound();
  const t = translator(locale);
  return (
    <html lang={locale}>
      <body>
        <header className="nav">
          <Link href={`/${locale}`} className="logo">Relay</Link>
          <nav aria-label="Main">
            <Link href={`/${locale}`}>{t("nav.home")}</Link>
            <Link href={`/${locale}/pricing`}>{t("nav.pricing")}</Link>
            <span className="locales">
              {LOCALES.map((l) => (
                <Link key={l} href={`/${l}`} lang={l} aria-current={l === locale ? "page" : undefined}>{l.toUpperCase()}</Link>
              ))}
            </span>
          </nav>
        </header>
        {children}
        <footer>{t("footer")}</footer>
        <Review locale={locale} />
      </body>
    </html>
  );
}
