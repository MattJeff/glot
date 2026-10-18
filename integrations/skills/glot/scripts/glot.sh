#!/bin/sh
# Run glot: the installed binary when present, else the npm package. Same arguments.
if command -v glot >/dev/null 2>&1; then
  exec glot "$@"
fi
if command -v npx >/dev/null 2>&1; then
  exec npx -y glot-i18n "$@"
fi
echo "glot not found: install it (npm i -g glot-i18n, brew install mattjeff/glot/glot) or install Node.js for npx" >&2
exit 127
