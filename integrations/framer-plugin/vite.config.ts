import { defineConfig } from "vite";
import mkcert from "vite-plugin-mkcert";
import framer from "vite-plugin-framer";

// Same setup as `npm create framer-plugin`: HTTPS dev server (Framer loads the plugin from https://localhost:5173).
export default defineConfig({ plugins: [mkcert(), framer()] });
