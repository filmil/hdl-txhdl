#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# An image records no path of the machine it was built on: not the
# Bazel sandbox, the execution root or the repository cache, which
# change from run to run and from machine to machine, so that two
# builds of one tree give one `.elf` and one `.config` (issue 711).
set -euo pipefail

elf=$1
config=$2
where='processwrapper-sandbox|/execroot/|/sandbox/|/bazel/|/\.cache/'
status=0
for f in "$elf" "$config"; do
  n=$( { grep -a -o -E "[^[:space:][:cntrl:]]*($where)[^[:space:][:cntrl:]]*" \
    "$f" || true; } | sort -u | wc -l)
  if [[ $n -ne 0 ]]; then
    echo "$f names $n path(s) of the machine that built it:" >&2
    grep -a -o -E "[^[:space:][:cntrl:]]*($where)[^[:space:][:cntrl:]]*" \
      "$f" | sort -u | head -5 >&2
    status=1
  fi
done
if grep -q '(/' "$config"; then
  echo "$config names a module by an absolute path:" >&2
  grep '(/' "$config" >&2
  status=1
fi
[[ $status -eq 0 ]] && echo "no machine paths in the image"
exit $status
