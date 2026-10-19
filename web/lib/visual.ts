import type {
  PreviewSettings,
  Project,
  VisualIssue,
  VisualKind,
  VisualReport,
} from "@/lib/api";

/** Same rule as packages/visual-qa `buildUrl`: the source locale drops its `/{locale}` segment (next-intl `as-needed`)
 *  or its `?lang={locale}` param (`/pricing?lang=en`, bare `/pricing` for the source). */
// `keepSourcePrefix`: iframes side by side share one cookie jar, so an unprefixed source URL can render
// whatever locale another iframe just stored in the locale cookie; an explicit `/en/…` pins it.
export function buildUrl(
  base: string,
  pattern: string,
  locale: string,
  sourceLocale: string,
  route: string,
  keepSourcePrefix = false,
) {
  const pat =
    locale === sourceLocale && !keepSourcePrefix
      ? pattern
          .replace(/[?&][^?&=#]+=\{locale\}/, "")
          .replace(/^([^?]*)&/, "$1?")
          .replace(/\/?\{locale\}/, "")
      : pattern;
  let path = pat
    .replace("{locale}", locale)
    .replace("{route}", route)
    .replace(/\/{2,}/g, "/");
  if (path.length > 1) path = path.replace(/\/$/, "");
  return base.replace(/\/$/, "") + (path.startsWith("/") ? path : `/${path}`);
}

export const parseViewport = (vp: string) => {
  const [w, h] = vp.split("x").map(Number);
  return { w: w || 1280, h: h || 800 };
};

/** Preview target: project settings first, then whatever the last visual QA run used. */
export function previewTarget(
  settings: PreviewSettings | undefined,
  report: VisualReport | undefined,
) {
  const o = report?.report.options;
  return {
    base: settings?.base_url || o?.baseUrl || "",
    pattern: settings?.base_url
      ? settings.url_pattern
      : (o?.urlPattern ?? settings?.url_pattern ?? "/{locale}{route}"),
    routes: [
      ...new Set([...(settings?.routes ?? []), ...(o?.routes ?? []), "/"]),
    ],
    viewports: {
      desktop:
        report?.report.options.viewports.find(
          (v) => parseViewport(v).w >= 900,
        ) ?? "1280x800",
      mobile:
        report?.report.options.viewports.find(
          (v) => parseViewport(v).w < 900,
        ) ?? "390x844",
    },
  };
}

export const KIND_STYLE: Record<VisualKind, string> = {
  overflow: "bg-destructive/10 text-destructive",
  clamp: "bg-muted text-muted-foreground",
  overlap: "bg-destructive/10 text-destructive",
  untranslated: "bg-warning/15 text-warning",
  tofu: "bg-warning/15 text-warning",
  rtl: "bg-info/12 text-info",
};

export const issueLabel = (i: VisualIssue) => i.key ?? i.selector;

/** Same screen + same element + same problem: the identity used to diff two reports (new / fixed). */
export const issueSig = (i: VisualIssue) =>
  `${i.kind}|${i.locale}|${i.route}|${i.viewport}|${issueLabel(i)}`;

/**
 * Expected, not a bug: `clamp` is truncation the CSS asks for (ellipsis, line-clamp), and "untranslated" on the
 * source locale compares a page with itself (reports uploaded before visual-qa learnt the project's source).
 */
export const isExpected = (i: VisualIssue, sourceLocale: string) =>
  i.kind === "clamp" ||
  (i.kind === "untranslated" && i.locale === sourceLocale);

const sh = (s: string) =>
  /^[\w/.,:@=-]+$/.test(s) ? s : `'${s.replace(/'/g, "'\\''")}'`;

/** The visual-qa CLI invocation for this project: locales, routes and site come from the project, not CLI defaults. */
export function runCommand(
  p: Project,
  server: string,
  target: ReturnType<typeof previewTarget>,
) {
  const routes = target.routes.length ? target.routes : ["/"];
  return [
    "GLOT_TOKEN=glot_… npx @glot-i18n/visual-qa",
    `--base-url ${sh(target.base || "http://localhost:3000")}`,
    `--source-locale ${p.source_locale} --locales ${p.target_locales.join(",") || p.source_locale}`,
    `--routes ${sh(routes.join(","))}`,
    ...(target.pattern !== "/{locale}{route}"
      ? [`--url-pattern ${sh(target.pattern)}`]
      : []),
    `--upload ${server} --project ${p.slug}`,
  ].join(" \\\n  ");
}
