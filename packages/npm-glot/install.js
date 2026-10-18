#!/usr/bin/env node
// Downloads the glot binary for this platform from the GitHub release matching
// package.json's version, verifies it against checksums.json, extracts it to vendor/.
// Runs at postinstall and, if that was skipped (--ignore-scripts, pnpm), on first run.
"use strict";
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const { version } = require("./package.json");
const checksums = require("./checksums.json");

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};
const exe = process.platform === "win32" ? "glot.exe" : "glot";
const binPath = path.join(__dirname, "vendor", exe);

async function ensure() {
  if (process.env.GLOT_BINARY) return process.env.GLOT_BINARY; // bring your own build
  if (fs.existsSync(binPath)) return binPath;

  const key = `${process.platform}-${process.arch}`;
  const target = TARGETS[key];
  if (!target) {
    throw new Error(`glot has no prebuilt binary for ${key}. Build it: cargo install --git https://github.com/MattJeff/glot glot-cli`);
  }
  const asset = `glot-${target}.${process.platform === "win32" ? "zip" : "tar.gz"}`;
  const expected = checksums[asset];
  if (!expected) throw new Error(`no pinned checksum for ${asset} in checksums.json`);
  // GLOT_DOWNLOAD_BASE: mirror/proxy hosting the same file names (checksums still enforced).
  const base = process.env.GLOT_DOWNLOAD_BASE || `https://github.com/MattJeff/glot/releases/download/v${version}`;
  const url = `${base.replace(/\/$/, "")}/${asset}`;

  const res = await fetch(url);
  if (!res.ok) throw new Error(`downloading ${url}: HTTP ${res.status}`);
  const buf = Buffer.from(await res.arrayBuffer());
  const actual = crypto.createHash("sha256").update(buf).digest("hex");
  if (actual !== expected) {
    throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}. Refusing to install.`);
  }

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "glot-"));
  try {
    const archive = path.join(tmp, asset);
    fs.writeFileSync(archive, buf);
    // bsdtar (macOS, Windows 10+) and GNU tar both read tar.gz; bsdtar also reads zip.
    execFileSync("tar", ["-xf", archive, "-C", tmp, exe], { stdio: "ignore" });
    fs.mkdirSync(path.dirname(binPath), { recursive: true });
    fs.copyFileSync(path.join(tmp, exe), binPath + ".tmp");
    fs.chmodSync(binPath + ".tmp", 0o755);
    fs.renameSync(binPath + ".tmp", binPath); // atomic: concurrent npx runs never see a partial file
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  return binPath;
}

module.exports = { ensure };

if (require.main === module) {
  ensure().then(
    (p) => console.error(`glot ${version} installed (${p})`),
    // ponytail: never break `npm install`; bin/glot.js retries and reports the error on first run.
    (e) => console.error(`glot: binary not installed yet (${e.message}); will retry on first run.`),
  );
}
