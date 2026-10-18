import Link from "next/link";
import { getLocale, getTranslations } from "next-intl/server";
import { href } from "./href";

export default async function Home() {
  const [locale, t] = [await getLocale(), await getTranslations("home")];
  return (
    <main>
      <section className="hero">
        <h1 id="hero-title">{t("hero.title")}</h1>
        <p id="hero-subtitle">{t("hero.subtitle")}</p>
        <Link href={href(locale, "/signup")} className="btn btn-lg" id="hero-cta">{t("hero.cta")}</Link>
      </section>
      <section className="features">
        <h2>{t("features.title")}</h2>
        <ul>
          <li>{t("features.draft")}</li>
          <li>{t("features.chase")}</li>
          <li>{t("features.tax")}</li>
        </ul>
      </section>
    </main>
  );
}
