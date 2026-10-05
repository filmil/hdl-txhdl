#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The entry points a Khronos header declares that the library does not
# implement, as C functions that set an error and return zero, so that
# every program written against the header links and is told what it
# asked for is not there. They are written from the header itself, and
# an entry point is left out when the Rust source defines it, so there
# is no list to keep: implementing one takes its stub away.
#
# For GL ES (issue 1224) the error is GL_INVALID_OPERATION; for EGL
# (issue 996) it is EGL_BAD_MATCH.
#
#   stubs.sh GLES/gl.h capi/lib.rs gl  > stubs.c
#   stubs.sh EGL/egl.h egl/lib.rs  egl > egl_stubs.c
set -euo pipefail

header=$1
lib=$2
api=$3
case "$api" in
  gl)
    include='#include <GLES/gl.h>'
    record='extern void gles_record_error(GLenum error);'
    decl='^GL_API .* GL_APIENTRY gl'
    entry='GL_APIENTRY'
    fail='gles_record_error(GL_INVALID_OPERATION);'
    ;;
  egl)
    include='#include <EGL/egl.h>'
    record='extern void egl_record_error(EGLint error);'
    decl='^EGLAPI .* EGLAPIENTRY egl'
    entry='EGLAPIENTRY'
    fail='egl_record_error(EGL_BAD_MATCH);'
    ;;
  *)
    echo "stubs.sh: no API $api" >&2
    exit 1
    ;;
esac
have=$(grep -o "extern \"C\" fn ${api}[A-Z][A-Za-z0-9]*" "$lib" | sed 's/.* fn //' | sort -u)

cat <<HEAD
/* SPDX-License-Identifier: Apache-2.0 */
/* Written by gles/capi/stubs.sh from $(basename "$header"); do not edit. */
$include

$record
HEAD

grep "$decl" "$header" | while IFS= read -r line; do
  name=$(sed -E "s/.*$entry (${api}[A-Za-z0-9]+) .*/\1/" <<<"$line")
  if grep -qx "$name" <<<"$have"; then
    continue
  fi
  ret=$(sed -E "s/^[A-Z_]+ (.*) $entry.*/\1/" <<<"$line")
  printf '\n%s {\n  %s\n' "${line%;}" "$fail"
  if [[ "$ret" != "void" ]]; then
    printf '  return 0;\n'
  fi
  printf '}\n'
done
