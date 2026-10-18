// Server Component: `getGlotTranslations` instead of `getTranslations` so the text follows the on-device fallback.
import { getGlotTranslations } from "@glot/runtime/next-intl/server";
import { Counter } from "./counter";
import { Buy } from "./buy";
import { GlotCopy } from "@glot/runtime/next-intl";

export default async function Page() {
  const t = await getGlotTranslations("home");
  return (
    <main>
      <h1 data-glot-key="home.title">{t("title", { name: "Ana" })}</h1>
      <p id="files">{t("files", { n: 3 })}</p>
      <p id="server">{t("server")}</p>
      {/* client-only fallback: masked (this block only) until the assignment is known, ≤ 300 ms */}
      <p id="tagline"><GlotCopy k="home.tagline" /></p>
      <Counter />
      <Buy />
    </main>
  );
}
