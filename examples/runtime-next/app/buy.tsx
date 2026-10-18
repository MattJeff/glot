"use client";
// Conversion with revenue: credited to the experiment / cell / arm the edge assigned (x-glot-xp → sessionStorage).
import { init, track } from "@glot/insights";
import { useTranslations } from "next-intl";

export function Buy() {
  const t = useTranslations("counter");
  return (
    <button id="buy" onClick={async () => { await init({ key: "pk_demo" }); track("purchase", { value: 49, currency: "EUR" }); }}>
      {t("buy")}
    </button>
  );
}
