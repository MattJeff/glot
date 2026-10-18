import { getTranslations } from "next-intl/server";
import { SignupForm } from "./form";

export default async function Signup() {
  const t = await getTranslations("signup");
  return (
    <main>
      <h1>{t("title")}</h1>
      <SignupForm />
    </main>
  );
}
