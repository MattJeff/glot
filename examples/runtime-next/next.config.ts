import createNextIntlPlugin from "next-intl/plugin";

// NEXT_DIST_DIR: lets a test build next to a running `next dev` without clobbering `.next`.
export default createNextIntlPlugin()({ distDir: process.env.NEXT_DIST_DIR || ".next" });
