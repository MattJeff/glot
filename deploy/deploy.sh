#!/usr/bin/env bash
# Deploy glot from your Mac (repo root or anywhere):
#   GLOT_HOST=1.2.3.4 deploy/deploy.sh             # rsync code, build on the server, up, health check, auto-rollback
#   GLOT_HOST=1.2.3.4 deploy/deploy.sh rollback    # back to the images of the previous deploy
# Images are built ON the server by default (native arch). On small boxes (1 vCPU / 2 GB) set
#   GLOT_IMAGE_TAG=<git sha>   to pull ghcr.io/mattjeff/glot-{server,web}:<sha> built by .github/workflows/images.yml.
# GLOT_SSH_KEY=~/.ssh/glot_hetzner selects the SSH key.
set -euo pipefail
: "${GLOT_HOST:?set GLOT_HOST (server IP or name)}"
H="${GLOT_SSH_USER:-glot}@$GLOT_HOST"
SSH=(ssh); [ -n "${GLOT_SSH_KEY:-}" ] && SSH=(ssh -i "$GLOT_SSH_KEY")
ssh() { command "${SSH[@]}" "$@"; }
REGISTRY="${GLOT_REGISTRY:-ghcr.io/mattjeff}"
cd "$(dirname "$0")/.."
remote() { ssh "$H" "set -e; cd glot/deploy; $1"; }

rollback() {
  remote 'for s in server web; do docker tag glot-$s:previous glot-$s:latest; done
    docker compose -f compose.prod.yml up -d --wait'
  echo "rolled back to previous images"
}
[ "${1:-}" = rollback ] && { rollback; exit; }

ssh "$H" mkdir -p glot
rsync -az --delete -e "${SSH[*]}" \
  --exclude .git --exclude target --exclude node_modules --exclude .next --exclude .DS_Store \
  --exclude '*.tsbuildinfo' --exclude /deploy/.env \
  ./ "$H:glot/"

remote 'test -f .env || { echo "missing glot/deploy/.env on the server: cp .env.example .env, fill it, chmod 600 .env" >&2; exit 1; }
  for s in server web; do
    if docker image inspect glot-$s:latest >/dev/null 2>&1; then docker tag glot-$s:latest glot-$s:previous; fi
  done
  if [ -n "'"${GLOT_IMAGE_TAG:-}"'" ]; then
    for s in server web; do
      docker pull '"$REGISTRY"'/glot-$s:'"${GLOT_IMAGE_TAG:-}"' && docker tag '"$REGISTRY"'/glot-$s:'"${GLOT_IMAGE_TAG:-}"' glot-$s:latest
    done
  else
    docker compose -f compose.prod.yml build
  fi
  docker compose -f compose.prod.yml up -d --wait   # glot-server runs migrations at startup
  docker image prune -f >/dev/null
  mkdir -p ~/backups
  ( crontab -l 2>/dev/null | grep -v "# glot-backup" || true
    echo "17 3 * * * $HOME/glot/deploy/backup.sh >> $HOME/backups/backup.log 2>&1 # glot-backup" ) | crontab -'

echo "health check (up to 2 min, first deploy waits for Let's Encrypt)..."
for _ in $(seq 24); do
  deploy/status.sh >/dev/null 2>&1 && { deploy/status.sh; echo "deployed"; exit 0; }
  sleep 5
done
deploy/status.sh || true
if ssh "$H" docker image inspect glot-server:previous >/dev/null 2>&1; then
  echo "health check failed: rolling back" >&2; rollback
else
  echo "health check failed (first deploy, nothing to roll back to): docker compose logs on the server" >&2
fi
exit 1
