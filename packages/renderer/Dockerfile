# glot renderer (SPA snapshots for glot Edge). Built from the repo root: see deploy/compose.prod.yml.
# Small-VPS image: Node slim + chromium-headless-shell only (`--only-shell`: no full Chrome, no Firefox/WebKit),
# instead of the 2 GB official Playwright image. Node >= 22.18 strips TS types, the sources run as is.
FROM node:24-bookworm-slim
WORKDIR /app
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
COPY packages/renderer/package.json packages/renderer/pnpm-lock.yaml ./
# Playwright's version comes from the lockfile; `install-deps` pulls the shell's system libraries only.
RUN npm install -g pnpm@9.15.9 \
 && pnpm install --prod --frozen-lockfile \
 && pnpm exec playwright install --with-deps --only-shell chromium \
 && rm -rf /var/lib/apt/lists/* /root/.npm /root/.cache /root/.local/share/pnpm \
 && mkdir /data && chown node:node /data
COPY packages/renderer/src ./src
USER node
ENV RENDERER_DIR=/data NODE_ENV=production
EXPOSE 8795
CMD ["node", "src/server.ts"]
