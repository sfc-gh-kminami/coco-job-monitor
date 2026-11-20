#!/bin/sh
INPUT=$(/bin/cat 2>/dev/null) || exit 0
[ -z "$INPUT" ] && exit 0

PAYLOAD=$(printf '%s' "$INPUT" | /usr/bin/jq -c '{
  session_id: (.session_id // ""),
  cwd: (.cwd // ""),
  hook_event_name: (.hook_event_name // ""),
  tool_name: (.tool_name // ""),
  tool_use_id: (.tool_use_id // ""),
  agent_id: (.agent_id // ""),
  agent_type: (.agent_type // ""),
  notification_type: (.notification_type // "")
}' 2>/dev/null) || exit 0

[ -z "$PAYLOAD" ] && exit 0

/usr/bin/curl -sS --connect-timeout 0.15 --max-time 0.35 \
  -X POST \
  -H "Content-Type: application/json" \
  --data-binary "$PAYLOAD" \
  "http://127.0.0.1:47823/event" >/dev/null 2>&1 || true

exit 0
