import Link from "next/link";
import { getLocale, getTranslations } from "next-intl/server";
import { href } from "../href";

export default async function Pricing() {
  const [locale, t] = [await getLocale(), await getTranslations("pricing")];
  const plans = [["starter", 9], ["team", 39]] as const;
  return (
    <main>
      <h1>{t("title")}</h1>
      <div className="plans">
        {plans.map(([p, price]) => (
          <article key={p} className="plan">
            <h2>{t(p)}</h2>
            <p className="price">€{price} <small>{t("month")}</small></p>
            <p>{t(`${p}Body`)}</p>
            <Link href={href(locale, "/signup")} className="btn">{t("choose", { plan: t(p) })}</Link>
          </article>
        ))}
      </div>
    </main>
  );
}
