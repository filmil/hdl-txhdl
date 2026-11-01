#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# Fails if an ELF holds a path of the sandbox it was built in (issue
# 2023): the same sources must give the same bytes wherever they are
# built, so no output may name the build's own directory. It runs on
# OpenSBI's ELF and on the FireSim payload, OpenSBI with the kernel
# inside.
#
#   no_build_paths_test.sh <elf>...
set -euo pipefail
if [ $# -eq 0 ]; then
  echo "no_build_paths_test.sh: no ELF to check" >&2
  exit 2
fi
pat='/(execroot|sandbox)/|[^.[:space:][:cntrl:]]/\.[sp]out/'
bad=0
for elf in "$@"; do
  # grep -a reads the binary file as text. -q decides, so no pipe can
  # hide a match; the paths are printed apart, as printable runs.
  # Besides Bazel's own directories, an absolute path to build.sh's
  # output directories, .sout and .pout: the mapped ones start "./".
  if grep -a -q -E "$pat" "$elf"; then
    echo "$elf names a build directory:" >&2
    grep -a -o -E "[[:graph:]]*($pat)[[:graph:]]*" "$elf" |
      sed -n 1,3p >&2 || true
    bad=1
  fi
done
exit $bad
