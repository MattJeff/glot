// Server Component: `getGlotTranslations` instead of `getTranslations` so the text follows the on-device fallback.
import { getGlotTranslations } from "@glot/runtime/next-intl/server";
import { Counter } from "./counter";

export default async function Page() {
  const t = await getGlotTranslations("home");
  return (
    <main>
      <h1>{t("title", { name: "Ana" })}</h1>
      <p id="files">{t("files", { n: 3 })}</p>
      <p id="server">{t("server")}</p>
      <Counter />
    </main>
  );
}
