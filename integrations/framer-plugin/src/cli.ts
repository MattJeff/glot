// Server-side sync through Framer's Server API (`framer-api`, Node 22+): no editor open, runs from cron / CI.
//
//   FRAMER_API_KEY=… GLOT_TOKEN=glot_… node src/cli.ts <framer project URL> <glot project slug> [--no-translate] [--force] [--publish]
//
// GLOT_URL defaults to https://app.glotlang.com. --publish publishes the Framer project and deploys it to production.
import { connect } from "framer-api";
import { glotClient, sync } from "./sync.ts";

const args = process.argv.slice(2);
const [projectUrl, slug] = args.filter((a) => !a.startsWith("--"));
const flag = (f: string) => args.includes(f);
const token = process.env.GLOT_TOKEN;
if (!projectUrl || !slug || !token || !process.env.FRAMER_API_KEY) {
  console.error("usage: FRAMER_API_KEY=… GLOT_TOKEN=glot_… node src/cli.ts <framer project URL> <glot project slug> [--no-translate] [--force] [--publish]");
  process.exit(2);
}

const framer = await connect(projectUrl, process.env.FRAMER_API_KEY);
try {
  const glot = glotClient({ base: process.env.GLOT_URL ?? "https://app.glotlang.com", token, project: slug });
  const r = await sync(framer, glot, {
    translate: !flag("--no-translate"),
    force: flag("--force"),
    onProgress: (m) => console.log(m),
  });
  console.log(
    `${r.sources} strings (${r.created} new, ${r.updated} changed in glot), ${r.written} translations written, ${r.skipped} skipped`,
  );
  if (r.unmatchedLocales.length) console.log(`Framer locales without a glot target locale: ${r.unmatchedLocales.join(", ")}`);
  for (const e of r.errors) console.error(`error: ${e}`);
  if (flag("--publish") && r.written) {
    const { deployment } = await framer.publish();
    await framer.deploy(deployment.id);
    console.log("Published and deployed.");
  }
  process.exitCode = r.errors.length ? 1 : 0;
} finally {
  await framer.disconnect();
}
