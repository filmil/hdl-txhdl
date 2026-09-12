#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Brings up Vivado's hw_server on the machine the board is attached to,
# and a tunnel from a local port to it, so that a programming target
# here can connect to localhost. Idempotent: a running server is left
# running, a standing tunnel is left standing.
#
#   hw_server [--server=HOST] [--remote-port=3121] [--local-port=3122]
#             [--idle=600]
#
# The server is TXHDL_BOARD_SERVER from the environment, which .bazelrc
# sets for `bazel run`, unless --server says otherwise.
set -euo pipefail

# --- begin runfiles.bash initialization v3 ---
f=bazel_tools/tools/bash/runfiles/runfiles.bash
# shellcheck disable=SC1090
source "${RUNFILES_DIR:-/dev/null}/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "${RUNFILES_MANIFEST_FILE:-/dev/null}" | cut -f2- -d' ')" 2>/dev/null || \
  source "$0.runfiles/$f" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  source "$(grep -sm1 "^$f " "$0.exe.runfiles_manifest" | cut -f2- -d' ')" 2>/dev/null || \
  { echo>&2 "ERROR: cannot find $f"; exit 1; }; f=
# --- end runfiles.bash initialization v3 ---

server="${TXHDL_BOARD_SERVER:-}"
rport=3121
lport=3122
idle=600
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --remote-port=*) rport="${a#*=}" ;;
    --local-port=*) lport="${a#*=}" ;;
    --idle=*) idle="${a#*=}" ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
[[ -n "$server" ]] || {
  echo "--server=HOST is required, or TXHDL_BOARD_SERVER in the environment" >&2
  exit 2
}

bin="$(rlocation _main/cpu/vreteno/board/remote/bin/hw_server)"
remote_sh="$(rlocation _main/cpu/vreteno/board/remote/hw_server_remote.sh)"
bundle="$(cd "$(dirname "$bin")/.." && pwd)"
dir='~/txhdl_hw_server'

echo "[hw_server] uploading the bundle to $server:$dir"
tar -C "$bundle" -chzf - bin lib \
  | ssh -o BatchMode=yes "$server" "mkdir -p $dir && tar -C $dir -xzf -"
scp -q -o BatchMode=yes "$remote_sh" "$server:$dir/hw_server_remote.sh"
ssh -o BatchMode=yes "$server" \
  "chmod +x $dir/hw_server_remote.sh && $dir/hw_server_remote.sh --port=$rport --idle=$idle"

tunnel="ssh -f -N -L $lport:localhost:$rport"
if pgrep -f "$tunnel .*$server" > /dev/null; then
  echo "[hw_server] tunnel localhost:$lport already standing"
else
  echo "[hw_server] opening the tunnel localhost:$lport -> $server:$rport"
  # shellcheck disable=SC2086
  $tunnel -o ExitOnForwardFailure=yes -o BatchMode=yes "$server"
fi
for _ in $(seq 1 20); do
  if (exec 3<>"/dev/tcp/localhost/$lport") 2>/dev/null; then
    echo "[hw_server] reachable at localhost:$lport"
    exit 0
  fi
  sleep 1
done
echo "[hw_server] localhost:$lport did not come up" >&2
exit 1
