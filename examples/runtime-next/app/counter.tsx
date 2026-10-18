"use client";
import { useState } from "react";
import { useTranslations } from "next-intl";

export function Counter() {
  const t = useTranslations("counter");
  const [count, setCount] = useState(0);
  return (
    <p>
      <span id="count">{t("label", { count })}</span> <button onClick={() => setCount(count + 1)}>{t("button")}</button>
    </p>
  );
}
