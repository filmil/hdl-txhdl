#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The entry points Khronos's GLES/gl.h declares that the library does
# not implement, as C functions that set GL_INVALID_OPERATION and
# return zero (issue 1224), so that every GL ES 1.1 program links and is
# told what it asked for is not there. They are written from gl.h
# itself, and an entry point is left out when lib.rs defines it, so
# there is no list to keep: implementing one in lib.rs takes its stub
# away.
#
#   stubs.sh GLES/gl.h capi/lib.rs > stubs.c
set -euo pipefail

gl_h=$1
lib=$2
have=$(grep -o 'extern "C" fn gl[A-Za-z0-9]*' "$lib" | sed 's/.* fn //' | sort -u)

cat <<'HEAD'
/* SPDX-License-Identifier: Apache-2.0 */
/* Written by gles/capi/stubs.sh from GLES/gl.h; do not edit. */
#include <GLES/gl.h>

extern void gles_record_error(GLenum error);
HEAD

grep '^GL_API .* GL_APIENTRY gl' "$gl_h" | while IFS= read -r line; do
  name=$(sed -E 's/.*GL_APIENTRY (gl[A-Za-z0-9]+) .*/\1/' <<<"$line")
  if grep -qx "$name" <<<"$have"; then
    continue
  fi
  ret=$(sed -E 's/^GL_API (.*) GL_APIENTRY.*/\1/' <<<"$line")
  printf '\n%s {\n  gles_record_error(GL_INVALID_OPERATION);\n' "${line%;}"
  if [[ "$ret" != "void" ]]; then
    printf '  return 0;\n'
  fi
  printf '}\n'
done
