#!/usr/bin/env bash
# Restore a dump made by backup.sh. DESTROYS the current database.
#   deploy/restore.sh /home/glot/backups/glot-20260101T031700Z.dump
set -euo pipefail
dump=${1:?usage: restore.sh <file.dump>}
[ -s "$dump" ] || { echo "no such dump: $dump" >&2; exit 1; }
dump=$(realpath "$dump")
cd "$(dirname "$0")"
c() { docker compose -f compose.prod.yml "$@"; }
c stop server web
c exec -T postgres dropdb -U glot --if-exists --force glot
c exec -T postgres createdb -U glot glot
c exec -T postgres pg_restore -U glot -d glot --no-owner --exit-on-error < "$dump"
c start server web
echo "restored $dump"
