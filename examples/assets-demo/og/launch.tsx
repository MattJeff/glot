// OG image rendered by @vercel/og (`new ImageResponse(<LaunchOg />, { width: 1200, height: 630 })`).
export default function LaunchOg() {
  return (
    <div
      style={{
        width: 1200,
        height: 630,
        padding: 80,
        display: "flex",
        flexDirection: "column",
        justifyContent: "center",
        backgroundColor: "#0b1020",
        color: "#ffffff",
        fontFamily: "Helvetica",
      }}
    >
      <div style={{ fontSize: 28, color: "#a5b4fc" }}>Acme 2.0 is live</div>
      <div style={{ fontSize: 72, fontWeight: 700, lineClamp: 2, marginTop: 16 }}>
        Ship localized apps in minutes, not months
      </div>
      <div style={{ fontSize: 30, color: "#cbd5e1", marginTop: 32 }}>
        Translations written by AI, checked by code.
      </div>
    </div>
  );
}
