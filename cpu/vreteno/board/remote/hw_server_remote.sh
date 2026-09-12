#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs on the machine the board is attached to: starts hw_server from
# the bundle beside this script, detached, with an idle timeout after
# which it exits by itself; a running one is left alone.
#
#   hw_server_remote.sh [--port=3121] [--idle=600]
set -euo pipefail
port=3121
idle=600
for a in "$@"; do
  case "$a" in
    --port=*) port="${a#*=}" ;;
    --idle=*) idle="${a#*=}" ;;
  esac
done
dir="$(cd "$(dirname "$0")" && pwd)"
if pgrep -f "$dir/bin/hw_server" > /dev/null; then
  echo "[$(hostname)] hw_server already running on port $port"
  exit 0
fi
chmod +x "$dir/bin/hw_server"
setsid nohup /usr/bin/env ld.so --library-path "$dir/lib" "$dir/bin/hw_server" \
  -stcp::"$port" -I "$idle" -L- -l jtag -l jtag2 -l events -l slave -l proxy \
  > "$dir/hw_server.log" 2>&1 < /dev/null &
sleep 2
if pgrep -f "$dir/bin/hw_server" > /dev/null; then
  echo "[$(hostname)] hw_server started on port $port, idle timeout ${idle}s"
else
  echo "[$(hostname)] hw_server did not start:" >&2
  cat "$dir/hw_server.log" >&2
  exit 1
fi
