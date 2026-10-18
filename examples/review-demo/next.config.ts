import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  agentRules: false,
  redirects: async () => [{ source: "/", destination: "/en", permanent: false }],
};

export default nextConfig;
