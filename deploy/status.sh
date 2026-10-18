#!/usr/bin/env bash
# Production health, from your Mac. Exit 1 if any public check fails (deploy.sh uses it to decide on rollback).
#   GLOT_HOST=1.2.3.4 deploy/status.sh            # server state (ssh) + public checks
#   GLOT_DOMAIN=example.com deploy/status.sh      # public checks only (no ssh)
set -uo pipefail
[ -n "${GLOT_SSH_KEY:-}" ] && ssh() { command ssh -i "$GLOT_SSH_KEY" "$@"; }
H="${GLOT_SSH_USER:-glot}@${GLOT_HOST:-}"
if [ -n "${GLOT_HOST:-}" ]; then
  D=${GLOT_DOMAIN:-$(ssh "$H" "sed -n 's/^GLOT_DOMAIN=//p' glot/deploy/.env")}
  ssh "$H" 'cd glot/deploy && docker compose -f compose.prod.yml ps --format "table {{.Service}}\t{{.Status}}"
    echo; docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}"
    echo; df -h / | tail -1; free -m | sed -n 2,3p
    echo; echo "last backup: $(ls -t ~/backups/*.dump 2>/dev/null | head -1)"
    echo; docker compose -f compose.prod.yml logs --since 1h server 2>&1 | grep -ciE "error|panic" | sed "s/^/server errors (1h): /"'
  echo
fi
D=${D:-${GLOT_DOMAIN:?set GLOT_HOST or GLOT_DOMAIN}}
P=${GLOT_PORT_SUFFIX:-}
fail=0
expect() { # expect CODE URL [curl args]  (no -k: an invalid certificate is a failure, code 000)
  local want=$1 url=$2 got; shift 2
  got=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 ${GLOT_CURL_OPTS:-} "$@" "$url")
  if [ "$got" = "$want" ]; then echo "ok   $got $url"; else echo "FAIL $got $url (want $want)"; fail=1; fi
}
expect 200 "https://api.$D$P/healthz"
expect 200 "https://$D$P/"
expect 200 "https://$D$P/docs/getting-started"
expect 200 "https://app.$D$P/login"
expect 200 "https://api.$D$P/.well-known/oauth-protected-resource"
expect 401 "https://api.$D$P/mcp" -X POST -H 'content-type: application/json' -d '{}'
[ -z "$P" ] && echo "cert $(echo | openssl s_client -connect "$D:443" -servername "$D" 2>/dev/null | openssl x509 -noout -enddate 2>/dev/null)"
exit $fail
