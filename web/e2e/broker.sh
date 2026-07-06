#!/usr/bin/env bash
# Launch the rendezvous broker for the federation E2E (M8). Port-scoped free so it
# never disturbs other control-plane instances: a blanket `pkill -x gaugewright-app`
# would kill a peer (or a dev) instance, so these launchers free only their own port.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO/target/debug/gaugewright-broker"
PORT="${BROKER_PORT:-7900}"

# Free only our port (broker is raw TCP; Playwright waits on the port).
(fuser -k "${PORT}/tcp" 2>/dev/null || lsof -ti "tcp:${PORT}" 2>/dev/null | xargs -r kill 2>/dev/null) || true
sleep 0.3

export GAUGEWRIGHT_BROKER_ADDR="127.0.0.1:${PORT}"
exec "$BIN"
