# glot renderer (SPA snapshots for glot Edge). Built from the repo root: see deploy/compose.prod.yml.
# Pinned to the Playwright release the package depends on (browsers baked in the image, Node >= 22.18 strips TS types).
FROM mcr.microsoft.com/playwright:v1.64.1-noble
WORKDIR /app
COPY packages/renderer/package.json packages/renderer/pnpm-lock.yaml ./
RUN npm install -g pnpm@9.15.9 && pnpm install --prod --frozen-lockfile
COPY packages/renderer/src ./src
USER pwuser
ENV RENDERER_DIR=/data NODE_ENV=production
EXPOSE 8795
CMD ["node", "src/server.ts"]
