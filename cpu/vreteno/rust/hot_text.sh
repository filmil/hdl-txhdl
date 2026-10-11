#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Writes `hot_text.ld`, the functions a program spends its instructions
# in, hottest first, as input-section patterns for the core's linker
# script (#1679). The core's instruction cache is direct-mapped, 16 KiB
# of 16-byte lines, so code that runs often and lies far apart in the
# text evicts itself; laid out together, it collides less.
#
#   bazel run //cpu/vreteno/rust:hot_text -- ELF PROFILE...
#
# ELF is the program as linked, and each PROFILE what the machine's
# `--profile` wrote for it (one per hart): lines of an address in hex
# and the instructions run there. The functions that ran 99.9% of the
# instructions are written, a pattern each. A Rust function's section
# is `.text.` and its mangled name, whose hash changes with the code, so
# the pattern stops before the hash; any other function's is its name.
set -euo pipefail

objdump=$1
shift
if [ $# -lt 2 ]; then
  echo "usage: hot_text ELF PROFILE..." >&2
  exit 2
fi
elf=$1
shift
out=${BUILD_WORKSPACE_DIRECTORY:-.}/cpu/vreteno/rust/hot_text.ld

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The functions, by address: address, size and name, from the symbol
# table's function entries.
"$objdump" -t "$elf" |
  awk '$3 == "F" && $4 ~ /^\.text/ {
         printf "%d %d %s\n", strtonum("0x" $1), strtonum("0x" $5), $NF
       }' |
  sort -n > "$tmp/syms"

# The instructions each function ran, the most first.
cat "$@" |
  awk -v S="$tmp/syms" '
    BEGIN {
      n = 0
      while ((getline l < S) > 0) {
        split(l, f, " ")
        a[n] = f[1]; z[n] = f[2]; name[n] = f[3]; n++
      }
    }
    {
      x = strtonum("0x" $1)
      lo = 0; hi = n - 1
      while (lo < hi) {
        m = int((lo + hi + 1) / 2)
        if (a[m] <= x) lo = m; else hi = m - 1
      }
      if (n > 0 && x >= a[lo] && x < a[lo] + z[lo]) by[name[lo]] += $2
    }
    END { for (s in by) printf "%d %s\n", by[s], s }' |
  sort -k1,1nr -k2 > "$tmp/ranked"

total=$(awk '{ t += $1 } END { print t + 0 }' "$tmp/ranked")
if [ "$total" -eq 0 ]; then
  echo "hot_text: no instructions of the profile lie in a function" >&2
  exit 1
fi

{
  cat <<'EOF'
    /* The functions a GL frame spends its instructions in, hottest
       first, for the core's linker script when
       //cpu/vreteno/rust:hot_first is set (#1679). Written by
       //cpu/vreteno/rust:hot_text from a profile of the canonical
       teapot; a pattern that matches nothing does nothing. */
EOF
  awk -v total="$total" '
    {
      sum += $1
      s = $2
      # A Rust symbol ends in its hash, 17h and sixteen hex digits, then
      # E, and perhaps a suffix LLVM added; the pattern stops before it.
      if (match(s, /17h[0-9a-f]+E(\..*)?$/)) s = substr(s, 1, RSTART + 2) "*"
      if (!(s in seen)) {
        seen[s] = 1
        printf "    *(.text.%s)\n", s
      }
      if (sum >= total * 0.999) exit
    }' "$tmp/ranked"
} > "$out"
echo "hot_text: wrote $out" >&2
