// Budget: loader + lazy app < 25 kB gzip; the loader alone (what every visitor pays) < 1.5 kB gzip.
import { readFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
const gz = (f) => gzipSync(readFileSync(new URL("../dist/" + f, import.meta.url))).length;
const loader = gz("review.global.js"), app = gz("review.app.global.js");
console.log(`loader ${loader} B gz · app ${app} B gz · total ${loader + app} B gz`);
if (loader > 1536 || loader + app > 25600) process.exit(1);
