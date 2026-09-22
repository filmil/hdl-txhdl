#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Puts the shim on the machine the board is wired to, as a systemd
# service with the one capability it needs: uploads the binary and a
# unit filled in for that machine to ~/txhdl_shim there, restarts the
# service when that can be done without a password, and otherwise
# prints the lines the user runs once as root, ending in the checks
# that prove the service is up and holds the capability.
#
#   deploy [--server=HOST] [--iface=fpga-a200t-eth0] [--dry-run]
#
# The server is TXHDL_BOARD_SERVER from the environment, which .bazelrc
# sets for `bazel run`, unless --server says otherwise. --dry-run
# prints the unit as it would be uploaded and the sequence, and
# touches nothing.
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
iface=fpga-a200t-eth0
dry=0
for a in "$@"; do
  case "$a" in
    --server=*) server="${a#*=}" ;;
    --iface=*) iface="${a#*=}" ;;
    --dry-run) dry=1 ;;
    *) echo "unknown argument: $a" >&2; exit 2 ;;
  esac
done
[[ -n "$server" ]] || {
  echo "--server=HOST is required, or TXHDL_BOARD_SERVER in the environment" >&2
  exit 2
}

bin="$(rlocation _main/tools/remote/shim/shim_/shim)"
unit_in="$(rlocation _main/tools/remote/shim/txhdl-shim.service.in)"
dir='~/txhdl_shim'
unit=txhdl-shim.service

# The unit names the user and home of the account on the server, which
# the server says; a dry run fills in this machine's.
if [[ "$dry" == 1 ]]; then
  ruser="$(id -un)"; rhome="$HOME"
else
  read -r ruser rhome < <(ssh -o BatchMode=yes "$server" 'echo "$(id -un) $HOME"')
fi
rendered="$(sed -e "s|@IFACE@|$iface|g" -e "s|@USER@|$ruser|g" -e "s|@HOME@|$rhome|g" "$unit_in")"

# What the user runs once, as root on the server, and the checks. The
# capability check reads the ambient set of the running process:
# CAP_NET_RAW is bit 13, so the word is 0000000000002000 and nothing
# else, or a directive in the unit was misspelled.
checks="systemctl is-active $unit
grep CapAmb /proc/\$(systemctl show -p MainPID --value $unit)/status
ss -ltn | grep 127.0.0.1:9797"
install_lines="sudo install -m 644 $rhome/txhdl_shim/$unit /etc/systemd/system/$unit
sudo systemctl daemon-reload
sudo systemctl enable --now $unit
$checks"

if [[ "$dry" == 1 ]]; then
  echo "[shim] would upload to $server:$dir: shim, and this unit:"
  echo "$rendered" | sed 's/^/    /'
  echo "[shim] then, once, as root on $server:"
  echo "$install_lines" | sed 's/^/    /'
  exit 0
fi

echo "[shim] uploading shim and $unit to $server:$dir"
ssh -o BatchMode=yes "$server" "mkdir -p $dir"
scp -q -o BatchMode=yes "$bin" "$server:$dir/shim"
echo "$rendered" | ssh -o BatchMode=yes "$server" "cat > $dir/$unit && chmod +x $dir/shim"

# A service already installed is restarted, if the account may do
# that without a password; the sudoers line for that, far narrower
# than any capability grant, is
#   $ruser ALL=(root) NOPASSWD: /usr/bin/systemctl restart $unit
if ssh -o BatchMode=yes "$server" "sudo -n systemctl restart $unit" 2>/dev/null; then
  echo "[shim] restarted $unit"
  ssh -o BatchMode=yes "$server" "$checks"
  exit 0
fi
echo "[shim] the service is not installed, or a restart wants a password."
echo "[shim] Once, as root on $server:"
echo "$install_lines" | sed 's/^/    /'
echo "[shim] The capability line must read CapAmb: 0000000000002000."
