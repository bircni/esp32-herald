#!/bin/sh
# Shows what Claude Code is doing as an item in the deskbuddy's feed.
#
#   tools/claude-hook.sh working   # a prompt was submitted
#   tools/claude-hook.sh waiting   # Claude needs your attention
#   tools/claude-hook.sh done      # Claude finished its turn
#   tools/claude-hook.sh clear
#
# Configuration (environment):
#   BUDDY_URL    base URL of the device, e.g. http://192.168.1.42
#                (the simulator listens on http://127.0.0.1:8080)
#   BUDDY_TOKEN  access token, if one is set on the device
#   BUDDY_ITEM   feed item id, default "claude"; set one per machine or
#                project to show several sessions side by side
#
# The script never fails and never waits longer than three seconds, so a
# switched-off buddy cannot hold up Claude Code.

url="${BUDDY_URL:-http://127.0.0.1:8080}"
item="${BUDDY_ITEM:-claude}"

case "${1:-}" in
  working) body='{"text":"Claude is working","color":"blue"}' ;;
  waiting) body='{"text":"Claude needs you","color":"yellow","wake":true,"beep":true}' ;;
  done)    body='{"text":"Claude is done","color":"green","ttl":900,"wake":true}' ;;
  clear)   body='' ;;
  *) echo "usage: $0 working|waiting|done|clear" >&2; exit 0 ;;
esac

if [ -n "$body" ]; then
  set -- -X PUT --data "$body" "$url/api/items/$item"
else
  set -- -X DELETE "$url/api/items/$item"
fi
if [ -n "${BUDDY_TOKEN:-}" ]; then
  set -- -H "X-Buddy-Token: $BUDDY_TOKEN" "$@"
fi
curl --silent --output /dev/null --max-time 3 "$@" || true
exit 0
