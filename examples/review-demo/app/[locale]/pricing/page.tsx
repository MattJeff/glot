import { isLocale, translator } from "../../i18n";

export default async function Pricing({ params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  if (!isLocale(locale)) return null;
  const t = translator(locale);
  return (
    <main>
      <h1>{t("pricing.title")}</h1>
      <div className="plans">
        {([["pricing.starter", 9], ["pricing.pro", 29]] as const).map(([k, price]) => (
          <div className="plan" key={k}>
            <h2>{t(k)}</h2>
            <p className="price">{price} € <small>{t("pricing.perMonth")}</small></p>
            <button className="btn btn-fixed" type="button">{t("pricing.choose")}</button>
          </div>
        ))}
      </div>
    </main>
  );
}
