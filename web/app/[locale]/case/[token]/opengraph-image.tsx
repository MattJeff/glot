import { ImageResponse } from "next/og";
import { getTranslations } from "next-intl/server";
import { fetchStory, languageName, signedPct } from "./data";

export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

// ponytail: English, like the site OG image (the default OG font has no CJK/Arabic glyphs).
export default async function OgImage({ params }: { params: Promise<{ token: string }> }) {
  const { token } = await params;
  const s = await fetchStory(token);
  const t = await getTranslations({ locale: "en", namespace: "launches" });
  const language = s ? languageName(s.locale, "en") : "";
  return new ImageResponse(
    (
      <div style={{ width: "100%", height: "100%", display: "flex", flexDirection: "column", justifyContent: "space-between", padding: 72, background: "#0b0f17", color: "#f5f6f8" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 16, fontSize: 36, fontWeight: 700 }}>
          <div style={{ width: 52, height: 52, borderRadius: 12, background: "#10b981", color: "#04120c", display: "flex", alignItems: "center", justifyContent: "center", fontSize: 28 }}>{"{ }"}</div>
          {"glot"}
        </div>
        {s ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
            <div style={{ fontSize: 30, color: "#a7adba" }}>{t("story.kicker")}</div>
            <div style={{ fontSize: 72, fontWeight: 700, letterSpacing: -2, lineHeight: 1.05 }}>{t("story.headline", { project: s.project, language })}</div>
            <div style={{ display: "flex", alignItems: "baseline", gap: 24, fontSize: 30, color: "#a7adba" }}>
              <span style={{ fontSize: 64, fontWeight: 700, color: s.lift_lo > 0 ? "#34d399" : "#f5f6f8" }}>{signedPct(s.lift, "en")}</span>
              <span>{`${s.goal} · ${t(`verdict.${s.verdict}`)}`}</span>
            </div>
          </div>
        ) : (
          <div style={{ fontSize: 64, fontWeight: 700 }}>{"glot"}</div>
        )}
        <div style={{ display: "flex", fontSize: 26, color: "#a7adba" }}>
          <span>{s && !s.causal ? t("story.caveatShort") : t("story.footerShort")}</span>
          <span style={{ marginLeft: "auto" }}>{"glotlang.com"}</span>
        </div>
      </div>
    ),
    size,
  );
}
