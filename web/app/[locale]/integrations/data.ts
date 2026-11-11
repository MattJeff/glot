// Platform guides: one data file (content/integrations/platforms.json), English steps as each platform names its
// menus; the page template around them is translated (messages → integrations). See the file's "note".
import { getTranslations } from "next-intl/server";
import type { Chrome } from "@/components/landing";
import data from "@/content/integrations/platforms.json";

export const CATEGORIES = ["site-builder", "e-commerce", "cms", "ai-builder", "lms", "help-center", "enterprise"] as const;
export type Category = (typeof CATEGORIES)[number];
export type Method = "connector" | "plugin" | "tag";

export type Platform = {
  id: string;
  name: string;
  category: Category;
  method: Method;
  summary: string;
  /** Native glot path (connector, plugin): steps in glot or the platform, and the docs anchor. */
  native?: { steps: string[]; doc: string };
  injection: { location: string; steps: string[]; plans: string; available: boolean };
  seo: { subdomain: boolean; how: string };
  faq: { q: string; a: string }[];
  docs: { label: string; url: string }[];
  verified: boolean;
  notes?: string;
};

export const PLATFORMS = [...(data.platforms as Platform[])].sort((a, b) => a.name.localeCompare(b.name));
export const platform = (id: string) => PLATFORMS.find((p) => p.id === id);

/** Header/footer labels from the home namespace (present in every catalog). */
export async function chrome(stars: number | null): Promise<Chrome> {
  const t = await getTranslations("home");
  return {
    skip: t("skip"),
    home: t("nav.home"),
    nav: t("nav.label"),
    docs: t("nav.docs"),
    pricing: t("nav.pricing"),
    openApp: t("nav.openApp"),
    stars: t("nav.githubStars", { count: stars ?? 0 }),
    tagline: t("footer.tagline"),
    npm: t("footer.npm"),
    license: t("footer.license"),
  };
}

export const ld = (x: unknown) => JSON.stringify(x).replace(/</g, "\\u003c");
