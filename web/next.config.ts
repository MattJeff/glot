import type { NextConfig } from "next";
import createNextIntlPlugin from "next-intl/plugin";

const API = process.env.GLOT_API_URL ?? "http://localhost:8790";

const nextConfig: NextConfig = {
  output: "standalone", // self-contained server for web/Dockerfile
  turbopack: {
    rules: {
      "*.css": {
        loaders: ["@tailwindcss/turbopack"],
        as: "*.css",
      },
    },
  },
  // The website tag loads on every page view of customers' sites: browsers reuse it for 5 minutes, then refresh
  // it in the background (the default for public files is max-age=0: one conditional request per page view).
  async headers() {
    return [{ source: "/widget/glot.js", headers: [{ key: "Cache-Control", value: "public, max-age=300, stale-while-revalidate=86400" }] }];
  },
  // Same origin as the dashboard so the `glot_session` cookie just works.
  async rewrites() {
    return [
      { source: "/api/:path*", destination: `${API}/api/:path*` },
      { source: "/cdn/:path*", destination: `${API}/cdn/:path*` },
    ];
  },
};

export default createNextIntlPlugin("./i18n/request.ts")(nextConfig);
