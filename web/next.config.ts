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
  // Same origin as the dashboard so the `glot_session` cookie just works.
  async rewrites() {
    return [
      { source: "/api/:path*", destination: `${API}/api/:path*` },
      { source: "/cdn/:path*", destination: `${API}/cdn/:path*` },
    ];
  },
};

export default createNextIntlPlugin("./i18n/request.ts")(nextConfig);
