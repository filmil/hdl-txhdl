#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# genext2fs as //firesim/ax7a200b/linux:blk_root runs it (issue 2020):
# a tree whose files were written at different times gives the same
# image once they are set to the epoch, and the image is ext2.
#
#   genext2fs_test.sh <genext2fs>
set -euo pipefail
gen=$1
t=${TEST_TMPDIR:-$(mktemp -d)}
image() { # <tree> <out> <mtime>
  local r=$1
  rm -rf "$r" && mkdir -p "$r/bin" "$r/etc" "$r/proc"
  echo 'a line to read back' >"$r/etc/hello"
  printf '#!/bin/sh\necho hi\n' >"$r/init"
  chmod 0755 "$r/init"
  find "$r" -exec touch -h -d "@$3" {} +
  # As the rule does: the files to the epoch, then the image.
  find "$r" -exec touch -h -d @0 {} +
  SOURCE_DATE_EPOCH=0 "$gen" -B 1024 -b 256 -N 32 -q -d "$r" "$2" >/dev/null
}
image "$t/a" "$t/a.ext2" 12345
image "$t/b" "$t/b.ext2" 99999
cmp "$t/a.ext2" "$t/b.ext2"
# The superblock is at byte 1024 and its magic, 0xEF53, at byte 56 of it.
magic=$(od -An -tx1 -j1080 -N2 "$t/a.ext2" | tr -d ' \n')
[ "$magic" = 53ef ] || {
  echo "not ext2: magic $magic" >&2
  exit 1
}
grep -q 'a line to read back' "$t/a.ext2"
echo "genext2fs: the same bytes, and ext2"
