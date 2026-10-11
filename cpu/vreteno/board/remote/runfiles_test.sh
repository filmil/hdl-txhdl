#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The board's remote scripts find their files by the repository's
# apparent name, `txhdl/...`, which the runfiles' repository mapping
# turns into `_main` in TxHDL's own build and into `txhdl+` in a module
# that depends on it (issue 1677). Each argument is a file a script
# looks up; each must resolve. And no script may look one up as
# `_main/...`, which only TxHDL's own build has.
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

fail=0
for path in "$@"; do
  got="$(rlocation "$path" || true)"
  if [[ -z "$got" || ! -e "$got" ]]; then
    echo "FAIL: $path does not resolve" >&2
    fail=1
  fi
done
for s in demos hw_server load serial; do
  script="$(rlocation "txhdl/cpu/vreteno/board/remote/$s.sh")"
  if grep -n 'rlocation _main/' "$script"; then
    echo "FAIL: $s.sh looks a file up under _main" >&2
    fail=1
  fi
done
[ "$fail" -eq 0 ] && echo "PASS: $# files, by the apparent name"
exit "$fail"
