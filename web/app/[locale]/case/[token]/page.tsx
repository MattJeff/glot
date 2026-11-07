import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { Logo } from "@/components/common";
import { LaunchStoryView } from "@/components/launch-story";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";
import { fetchStory, languageName, signedPct } from "./data";

type Props = { params: Promise<{ locale: string; token: string }> };

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { locale, token } = await params;
  const s = await fetchStory(token);
  if (!s) return {};
  const t = await getTranslations({ locale: locale as Locale, namespace: "launches" });
  const language = languageName(s.locale, locale);
  return {
    title: t("public.title", { language, project: s.project }),
    description: t("public.description", { verdict: t(`verdict.${s.verdict}`), lift: signedPct(s.lift, locale), goal: s.goal }),
    // ponytail: revocable links stay out of search indexes; publish a copy on the blog to rank.
    robots: { index: false },
  };
}

export default async function CaseStudy({ params }: Props) {
  const { locale, token } = await params;
  setRequestLocale(locale as Locale);
  const s = await fetchStory(token);
  if (!s) notFound();
  const t = await getTranslations("launches");
  return (
    <div className="flex min-h-screen flex-col">
      <header className="border-b">
        <div className="mx-auto flex h-14 max-w-4xl items-center px-4">
          <Link href="/">
            <Logo />
          </Link>
        </div>
      </header>
      <main id="main" className="mx-auto w-full max-w-4xl flex-1 px-4 py-10">
        <LaunchStoryView story={s} />
        <div className="mt-10 flex justify-center">
          <Button asChild>
            <Link href="/signup">{t("public.cta")}</Link>
          </Button>
        </div>
      </main>
    </div>
  );
}
