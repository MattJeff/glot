# shellcheck shell=bash
# Sourced by backup.sh / restore.sh (after .env). log, heartbeat ping, alert on failure.
log() { echo "$(date -u +%FT%TZ) $*"; }

# ping ok|fail: healthchecks.io-style heartbeat, if HEALTHCHECK_URL is set. Never fails the caller.
ping() {
  [ -n "${HEALTHCHECK_URL:-}" ] || return 0
  local u=$HEALTHCHECK_URL; [ "$1" = ok ] || u="$u/fail"
  curl -fsS -m 10 --retry 3 -o /dev/null "$u" || log "heartbeat ping failed"
}

# alert SUBJECT TEXT: email via Resend if RESEND_API_KEY is set, else the log line is all you get.
alert() {
  log "ALERT: $1 — $2"
  ping fail
  [ -n "${RESEND_API_KEY:-}" ] || return 0
  local from=${BACKUP_ALERT_FROM:-${EMAIL_FROM:-glot <noreply@${GLOT_DOMAIN}>}}
  local to=${BACKUP_ALERT_TO:-$ACME_EMAIL} text=${2//[\"\\]/}
  curl -fsS -m 20 -o /dev/null https://api.resend.com/emails \
    -H "Authorization: Bearer $RESEND_API_KEY" -H 'Content-Type: application/json' \
    -d "{\"from\":\"$from\",\"to\":[\"$to\"],\"subject\":\"[glot] $1\",\"text\":\"$text (host $(hostname))\"}" \
    || log "alert email failed"
}
