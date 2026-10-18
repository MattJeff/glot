#!/usr/bin/env bash
# Daily backup (cron 03:17 UTC, installed by deploy.sh). Runs on the server: deploy/backup.sh
#  1. pg_dump -Fc -> ~/backups/glot-TS.dump (plain, for fast local restores; 14 days)
#  2. bundle = dump + .env + caddy volumes (certificates), tar.gz, encrypted with age to
#     AGE_RECIPIENT (public key; the private key never lives on the server) -> glot-TS.tar.gz.age
#  3. if RCLONE_REMOTE: rclone copy (never sync) to daily/ (+ monthly/ on the 1st), verify
#     size+hash remotely, prune remote: daily 30 days, monthly 12 months.
# Any failure: exit != 0, alert (Resend email if RESEND_API_KEY) and HEALTHCHECK_URL/fail.
set -Eeuo pipefail
cd "$(dirname "$0")"
umask 077   # dumps and bundles hold secrets
set -a; . ./.env; set +a
. offsite/notify.sh
trap 'rc=$?; alert "backup FAILED" "backup.sh failed at line $LINENO (exit $rc), see ~/backups/backup.log"; exit $rc' ERR

[ -n "${AGE_RECIPIENT:-}" ] || { echo "set AGE_RECIPIENT (age public key) in deploy/.env" >&2; false; }
dir=${BACKUP_DIR:-$HOME/backups}
ts=$(date -u +%Y%m%dT%H%M%SZ)
start=$SECONDS
mkdir -p "$dir"
stage=$(mktemp -d "$dir/.stage-XXXX")
trap 'rm -rf "$stage"' EXIT

dump="$dir/glot-$ts.dump"
docker compose -f compose.prod.yml exec -T postgres pg_dump -U glot -d glot -Fc > "$dump.part"
# a dump pg_restore cannot list is not a backup
docker compose -f compose.prod.yml exec -T postgres pg_restore -l < "$dump.part" > /dev/null
mv "$dump.part" "$dump"   # no half-written dump ever looks valid
log "dump ok: $(du -h "$dump" | cut -f1) in $((SECONDS - start))s"

ln "$dump" "$stage/glot.dump"
cp .env "$stage/env"
# caddy:2 is already on the box and has tar; read-only mounts, the running caddy is untouched.
docker run --rm --entrypoint tar -v glot_caddy_data:/v/caddy_data:ro -v glot_caddy_config:/v/caddy_config:ro \
  caddy:2 -C /v -cf - caddy_data caddy_config > "$stage/volumes.tar"
bundle="$dir/glot-$ts.tar.gz.age"
tar -C "$stage" -czf - glot.dump env volumes.tar | age -r "$AGE_RECIPIENT" -o "$bundle.part"
mv "$bundle.part" "$bundle"
log "bundle ok: $bundle $(du -h "$bundle" | cut -f1)"

find "$dir" -maxdepth 1 \( -name 'glot-*.dump' -o -name 'glot-*.tar.gz.age' \) -mtime +"${BACKUP_KEEP_DAYS:-14}" -delete

if [ -n "${RCLONE_REMOTE:-}" ]; then
  name=$(basename "$bundle")
  dests=(daily); [ "$(date -u +%d)" = 01 ] && dests+=(monthly)
  for d in "${dests[@]}"; do
    # copy, not sync: an empty local dir must never wipe the remote.
    rclone copy "$bundle" "$RCLONE_REMOTE/$d/"
    # size + hash comparison of this one file against the remote copy
    rclone check --one-way --include "$name" "$dir" "$RCLONE_REMOTE/$d/"
  done
  # from the root with filters: a missing monthly/ dir is not an error
  rclone delete --min-age 30d --include 'daily/**' "$RCLONE_REMOTE/"
  rclone delete --min-age 365d --include 'monthly/**' "$RCLONE_REMOTE/"
  log "off-site ok: $RCLONE_REMOTE/{${dests[*]}}/$name"
else
  log "no RCLONE_REMOTE: encrypted bundle kept locally only"
fi

ping ok
log "backup ok in $((SECONDS - start))s"
