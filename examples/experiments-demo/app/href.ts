export const href = (locale: string, path: string) => (locale === "en" ? path : `/${locale}${path === "/" ? "" : path}`);
