import Link from "next/link";
import { isLocale, translator } from "../i18n";

export default async function Home({ params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  if (!isLocale(locale)) return null;
  const t = translator(locale);
  return (
    <main>
      <section className="hero">
        <h1 id="hero-title">{t("hero.title")}</h1>
        <p id="hero-subtitle">{t("hero.subtitle")}</p>
        <Link href={`/${locale}/pricing`} className="btn btn-fixed" id="hero-cta">{t("hero.cta")}</Link>
      </section>
      <section className="features">
        <h2 id="features-title">{t("features.title")}</h2>
        <ul>
          <li>{t("features.draft")}</li>
          <li>{t("features.chase")}</li>
          <li>{t("features.tax")}</li>
        </ul>
      </section>
    </main>
  );
}
