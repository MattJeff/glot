import { ImageResponse } from "next/og";
import { getTranslations } from "next-intl/server";

export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

// ponytail: English for every locale (the default OG font has no CJK/Arabic glyphs).
export default async function OgImage() {
  const tr = await getTranslations();
  const t = await getTranslations({ locale: "en", namespace: "landing" });
  const h = await getTranslations({ locale: "en", namespace: "home.hero" });
  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", justifyContent: "space-between", padding: 72, background: "#0b0f17", color: "#f5f6f8" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 16, fontSize: 36, fontWeight: 700 }}>
          <div style={{ width: 52, height: 52, borderRadius: 12, background: "#10b981", color: "#04120c", display: "flex", alignItems: "center", justifyContent: "center", fontSize: 28 }}>{"{ }"}</div>
          {tr("common.brand")}
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
          <div style={{ fontSize: 76, fontWeight: 700, letterSpacing: -2, lineHeight: 1.05 }}>{h("title")}</div>
          <div style={{ fontSize: 30, color: "#a7adba", maxWidth: 980 }}>{t("bench.title")}</div>
        </div>
        <div style={{ display: "flex", alignItems: "baseline", gap: 28, fontSize: 30, color: "#a7adba" }}>
          <span style={{ textDecoration: "line-through" }}>{"14.1%"}</span>
          <span style={{ fontSize: 64, fontWeight: 700, color: "#34d399" }}>{"0%"}</span>
          <span>{t("bench.glot")}</span>
          <span style={{ marginLeft: "auto" }}>{"glotlang.com"}</span>
        </div>
      </div>
    ),
    size,
  );
}
