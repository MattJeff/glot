import { useTranslations } from "next-intl";
import { Logo } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Link } from "@/i18n/navigation";

export default function NotFound() {
  const t = useTranslations("notFound");
  return (
    <main className="grid min-h-screen place-items-center px-4 text-center">
      <div>
        <Logo className="mb-6" />
        <h1 className="text-2xl font-semibold tracking-tight">{t("title")}</h1>
        <Button asChild variant="outline" className="mt-6">
          <Link href="/">{t("back")}</Link>
        </Button>
      </div>
    </main>
  );
}
