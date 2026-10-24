#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Where run.sh finds its tester (#1898). Started with runfiles that hold
# the tester, it gets past the lookup; started with another target's
# RUNFILES_DIR, as the nightly once started it, it finds none and says
# so with exit 2, before it reaches any server. The server named is one
# that cannot resolve, so nothing is ever sent.
#
#   run_test.sh   (data: //eth/hil:run)
set -uo pipefail
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

run="$(rlocation _main/eth/hil/run)"
[[ -x "$run" ]] || { echo "FAIL: no run in the runfiles" >&2; exit 1; }
args=(--server=nohost.invalid --mode=count --seconds=1)

# Another target's runfiles: the library, and nothing else.
foreign="$(mktemp -d)"
trap 'rm -rf "$foreign"' EXIT
mkdir -p "$foreign/bazel_tools/tools/bash/runfiles"
cp "$(rlocation bazel_tools/tools/bash/runfiles/runfiles.bash)" \
  "$foreign/bazel_tools/tools/bash/runfiles/"
out="$(RUNFILES_DIR="$foreign" RUNFILES_MANIFEST_FILE= JAVA_RUNFILES= \
  "$run" "${args[@]}" 2>&1)"
rc=$?
if [[ $rc -ne 2 || "$out" != *"no tester in the runfiles"* ]]; then
  echo "FAIL: with foreign runfiles: exit $rc, said: $out" >&2
  exit 1
fi

# This test's own runfiles, which hold the tester: past the lookup, and
# on to the upload, which fails at the server that does not resolve.
out="$("$run" "${args[@]}" 2>&1)"
if [[ "$out" == *"no tester"* || "$out" != *"uploading the tester"* ]]; then
  echo "FAIL: with its own runfiles, said: $out" >&2
  exit 1
fi
echo "PASS: the tester is found in its own runfiles and refused in others"
