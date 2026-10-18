import type { PreviewSettings, VisualIssue, VisualKind, VisualReport } from "@/lib/api";

/** Same rule as packages/visual-qa `buildUrl`: the source locale drops its `/{locale}` segment (next-intl `as-needed`). */
// `keepSourcePrefix`: iframes side by side share one cookie jar, so an unprefixed source URL can render
// whatever locale another iframe just stored in the locale cookie; an explicit `/en/…` pins it.
export function buildUrl(base: string, pattern: string, locale: string, sourceLocale: string, route: string, keepSourcePrefix = false) {
  const pat = locale === sourceLocale && !keepSourcePrefix ? pattern.replace(/\/?\{locale\}/, "") : pattern;
  let path = pat.replace("{locale}", locale).replace("{route}", route).replace(/\/{2,}/g, "/");
  if (path.length > 1) path = path.replace(/\/$/, "");
  return base.replace(/\/$/, "") + (path.startsWith("/") ? path : `/${path}`);
}

export const parseViewport = (vp: string) => {
  const [w, h] = vp.split("x").map(Number);
  return { w: w || 1280, h: h || 800 };
};

/** Preview target: project settings first, then whatever the last visual QA run used. */
export function previewTarget(settings: PreviewSettings | undefined, report: VisualReport | undefined) {
  const o = report?.report.options;
  return {
    base: settings?.base_url || o?.baseUrl || "",
    pattern: settings?.base_url ? settings.url_pattern : (o?.urlPattern ?? settings?.url_pattern ?? "/{locale}{route}"),
    routes: [...new Set([...(settings?.routes ?? []), ...(o?.routes ?? []), "/"])],
    viewports: {
      desktop: report?.report.options.viewports.find((v) => parseViewport(v).w >= 900) ?? "1280x800",
      mobile: report?.report.options.viewports.find((v) => parseViewport(v).w < 900) ?? "390x844",
    },
  };
}

export const KIND_STYLE: Record<VisualKind, string> = {
  overflow: "bg-destructive/10 text-destructive",
  clamp: "bg-destructive/10 text-destructive",
  overlap: "bg-destructive/10 text-destructive",
  untranslated: "bg-warning/15 text-warning",
  tofu: "bg-warning/15 text-warning",
  rtl: "bg-info/12 text-info",
};

export const issueLabel = (i: VisualIssue) => i.key ?? i.selector;
