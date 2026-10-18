import { useEffect } from "react";
import type { OverlayOptions } from "./index.js";

/** `<GlotOverlay />` — mounts the overlay once on the client, dev only. Renders nothing. */
export function GlotOverlay(props: OverlayOptions) {
  useEffect(() => {
    if (process.env.NODE_ENV === "production") return;
    (window as any).GLOT_OVERLAY_MANUAL = true;
    import("./index.js").then((m) => m.init(props));
  }, []);
  return null;
}
declare const process: { env: Record<string, string | undefined> };
