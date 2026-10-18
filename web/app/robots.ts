import type { MetadataRoute } from "next";
import { SITE } from "@/lib/seo";

// AI crawlers are welcome: being cited in generated answers is an acquisition channel for a devtool.
export default function robots(): MetadataRoute.Robots {
  return {
    rules: {
      userAgent: ["*", "GPTBot", "OAI-SearchBot", "ChatGPT-User", "ClaudeBot", "Claude-User", "Claude-SearchBot", "PerplexityBot", "Perplexity-User", "Google-Extended"],
      allow: "/",
      disallow: ["/api/", "/cdn/", "/app", "/*/app", "/admin", "/*/admin", "/oauth/", "/*/oauth/"],
    },
    sitemap: `${SITE}/sitemap.xml`,
  };
}
