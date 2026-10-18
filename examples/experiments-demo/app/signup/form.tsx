"use client";
import { useState } from "react";
import { useTranslations } from "next-intl";
import { track } from "@glot/insights";

export function SignupForm() {
  const t = useTranslations("signup");
  const [done, setDone] = useState(false);
  if (done) return <p id="signup-done">{t("done")}</p>;
  return (
    <form onSubmit={(e) => { e.preventDefault(); track("signup"); setDone(true); }}>
      <label>{t("email")} <input type="email" name="email" required autoComplete="email" /></label>
      <button className="btn" type="submit">{t("submit")}</button>
    </form>
  );
}
