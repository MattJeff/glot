#!/usr/bin/env node
"use strict";
const { spawnSync } = require("node:child_process");
const { ensure } = require("../install.js");

ensure().then(
  (bin) => {
    // stdio inherited: `npx glot-i18n mcp` speaks MCP over this process's stdin/stdout.
    const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
    if (r.error) throw r.error;
    process.exit(r.status ?? (r.signal ? 1 : 0));
  },
  (e) => {
    console.error(`glot: ${e.message}`);
    process.exit(1);
  },
);
