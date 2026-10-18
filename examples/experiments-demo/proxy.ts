// Locale URLs (/fr, /de) + segmented copy experiments: the arm is decided here, before the page renders.
// Without GLOT_SERVER the app runs on its shipped catalogs only (same code path, no experiment).
import { glotProxy } from "@glot/edge/next";

const { GLOT_SERVER: server, GLOT_PROJECT: project = "relay", GLOT_KEY: key } = process.env;

export const proxy = glotProxy({
  source: "en",
  locales: ["fr", "de"],
  experiments: server && key ? { server, project, key } : undefined,
});
export const config = { matcher: ["/((?!_next|api|.*\\..*).*)"] };
