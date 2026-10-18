# glot-i18n

npm distribution of [glot](https://github.com/MattJeff/glot): AI-native i18n for SaaS (CLI + MCP server).

```sh
npx glot-i18n init          # detect stack, write glot.toml + agent rules
npx glot-i18n mcp           # MCP server on stdio
npm i -g glot-i18n && glot translate
```

On install (or on first run if install scripts are disabled), the prebuilt binary for your platform is downloaded from the matching GitHub release and checked against the SHA-256 pinned in `checksums.json`. A mismatch aborts.

Platforms: macOS arm64/x64, Linux arm64/x64 (glibc), Windows x64. Elsewhere: `cargo install --git https://github.com/MattJeff/glot glot-cli`.

Env: `GLOT_BINARY=/path/to/glot` uses your own build; `GLOT_DOWNLOAD_BASE=https://mirror/...` downloads from a mirror (checksums still enforced).

Maintainers: `scripts/sync-checksums.sh v0.X.Y` then bump `version` to match, then `npm publish`.
