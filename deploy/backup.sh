#!/usr/bin/env bash
# Daily Postgres dump (pg_dump -Fc = compressed), 14-day rotation, optional rclone off-site copy.
# Runs on the server: deploy/backup.sh   (cron installed by deploy.sh)
set -euo pipefail
cd "$(dirname "$0")"
set -a; . ./.env; set +a
dir=${BACKUP_DIR:-$HOME/backups}
mkdir -p "$dir"
f="$dir/glot-$(date -u +%Y%m%dT%H%M%SZ).dump"
docker compose -f compose.prod.yml exec -T postgres pg_dump -U glot -d glot -Fc > "$f.part"
mv "$f.part" "$f"   # no half-written dump ever looks valid
find "$dir" -name 'glot-*.dump' -mtime +"${BACKUP_KEEP_DAYS:-14}" -delete
if [ -n "${RCLONE_REMOTE:-}" ]; then
  # copy (not sync): an empty local dir must never wipe the remote. Remote keeps 30 days.
  docker run --rm --user "$(id -u):$(id -g)" -v "$HOME/.config/rclone:/config/rclone" -v "$dir:/data:ro" \
    rclone/rclone copy /data "$RCLONE_REMOTE"
  docker run --rm --user "$(id -u):$(id -g)" -v "$HOME/.config/rclone:/config/rclone" \
    rclone/rclone delete --min-age 30d "$RCLONE_REMOTE"
fi
echo "backup ok: $f ($(du -h "$f" | cut -f1))"
