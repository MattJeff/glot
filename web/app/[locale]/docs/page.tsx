import { redirect } from "@/i18n/navigation";
import type { Locale } from "@/i18n/routing";

export default async function DocsHome({ params }: PageProps<"/[locale]/docs">) {
  const { locale } = await params;
  redirect({ href: "/docs/getting-started", locale: locale as Locale });
}
