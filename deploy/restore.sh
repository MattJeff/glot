#!/usr/bin/env bash
# Restore a backup made by backup.sh.
#   restore.sh SOURCE              restore into production: DESTROYS the current database
#   restore.sh --drill [SOURCE]    restore into a throwaway postgres, count rows, delete it
# SOURCE: a local .dump or .tar.gz.age, `remote` (latest in RCLONE_REMOTE/daily),
#         `remote:daily/glot-....tar.gz.age`. Drill default: newest local .dump.
# .age sources need the private key: AGE_KEY=/path/to/key, or pipe it on stdin from your Mac:
#   ssh glot@SERVER 'glot/deploy/restore.sh --drill remote' < ~/.config/glot/backup-age.key
# The key is held in memory only, never written on the server.
set -Eeuo pipefail
cd "$(dirname "$0")"
set -a; . ./.env; set +a
. offsite/notify.sh
drill=; [ "${1:-}" = --drill ] && { drill=1; shift; }
dir=${BACKUP_DIR:-$HOME/backups}
src=${1:-}
if [ -z "$src" ] && [ -n "$drill" ]; then src=$(ls -t "$dir"/glot-*.dump | head -1); fi
[[ -z $src || $src == remote* || $src == /* ]] || src="$OLDPWD/$src"   # relative to the caller's dir
[ -n "$src" ] || { echo "usage: restore.sh [--drill] SOURCE" >&2; exit 2; }
start=$SECONDS
work=$(mktemp -d "$dir/.restore-XXXX")
cleanup() { if [ -n "${pg:-}" ]; then docker rm -f "$pg" > /dev/null 2>&1; fi; rm -rf "$work"; }
trap cleanup EXIT
[ -z "$drill" ] || trap 'rc=$?; alert "restore drill FAILED" "restore.sh --drill failed at line $LINENO (exit $rc)"; exit $rc' ERR

[ "$src" = remote ] && src="remote:daily/$(rclone lsf --files-only "$RCLONE_REMOTE/daily/" | sort | tail -1)"
case $src in
  remote:*)
    src=${src#remote:}
    rclone copy "$RCLONE_REMOTE/$(dirname "$src")/" "$work/" --include "$(basename "$src")"
    src="$work/$(basename "$src")"
    log "downloaded $src ($(du -h "$src" | cut -f1)) in $((SECONDS - start))s" ;;
esac
[ -s "$src" ] || { echo "no such backup: $src" >&2; exit 1; }

if [[ $src == *.age ]]; then
  if [ -n "${AGE_KEY:-}" ]; then key=$(cat "$AGE_KEY"); else key=$(cat); fi
  [[ $key == *AGE-SECRET-KEY-* ]] || { echo "need the age private key (AGE_KEY=file or on stdin)" >&2; exit 1; }
  age -d -i <(printf '%s\n' "$key") "$src" | tar -C "$work" -xzf -
  unset key
  dump="$work/glot.dump"
  log "decrypted: $(ls "$work" | tr '\n' ' ')"
else
  dump=$(realpath "$src")
fi

if [ -n "$drill" ]; then
  pg=glot-drill-$$
  docker run -d --name "$pg" --memory 512m -e POSTGRES_PASSWORD=drill postgres:17 > /dev/null
  # TCP only answers once the image's init phase is over
  until docker exec "$pg" pg_isready -h 127.0.0.1 -U postgres -q 2>/dev/null; do sleep 1; done
  docker exec -i "$pg" pg_restore -U postgres -d postgres --no-owner --no-acl --exit-on-error < "$dump"
  counts="select table_name, (xpath('/row/c/text()', query_to_xml(format('select count(*) c from public.%I', table_name), false, true, '')))[1]::text
          from information_schema.tables where table_schema = 'public' and table_type = 'BASE TABLE' order by table_name::text collate \"C\""
  docker exec "$pg" psql -U postgres -d postgres -Atc "$counts" > "$work/drill.txt"
  docker compose -f compose.prod.yml exec -T postgres psql -U glot -d glot -Atc "$counts" > "$work/live.txt" || true
  for t in users projects keys translations api_tokens; do
    grep -q "^$t|" "$work/drill.txt" || { echo "key table $t missing from restore" >&2; false; }
  done
  echo "table|restored|live now"; LC_ALL=C join -t'|' -a1 "$work/drill.txt" "$work/live.txt"
  ping ok
  log "drill ok: $(wc -l < "$work/drill.txt") tables, $(awk -F'|' '{s+=$2} END {print s}' "$work/drill.txt") rows, $((SECONDS - start))s total"
  exit 0
fi

c() { docker compose -f compose.prod.yml "$@"; }
c stop server web
c exec -T postgres dropdb -U glot --if-exists --force glot
c exec -T postgres createdb -U glot glot
c exec -T postgres pg_restore -U glot -d glot --no-owner --exit-on-error < "$dump"
c start server web
log "restored $src in $((SECONDS - start))s"
if [ -f "$work/env" ]; then
  keep="$dir/restored-$(date -u +%Y%m%dT%H%M%SZ)"; mkdir -m 700 "$keep"; mv "$work/env" "$work/volumes.tar" "$keep/"
  echo "bundle's .env and caddy volumes kept in $keep (see deploy-hetzner.md, Backups)"
fi
