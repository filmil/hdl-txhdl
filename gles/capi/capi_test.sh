#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The C entry points against the Rust API (issue 1224): draw.c, built
# against Khronos's <GLES/gl.h> and linked with the library, and
# draw_ref.rs draw the same scene, and every word of the two frames,
# the error after it, an unimplemented entry point's error and
# glGetString's version must be the same.
#
#   capi_test.sh <capi_draw> <capi_draw_ref>
set -euo pipefail
c=$("$1")
rust=$("$2")
if [[ "$c" != "$rust" ]]; then
  echo "the C entry points drew another frame than the Rust API:" >&2
  diff <(echo "$rust") <(echo "$c") >&2 || true
  exit 1
fi
# A clear of the colour and the depth with its depth plane's slot, the
# quad's two triangles, and the fan's two, each testing depth and
# blending with its slot: eight slots. An empty frame on both sides is not a pass.
# The first line, taken without a pipe: under pipefail, `echo | head`
# fails when head closes the pipe before echo is done (#1516).
first="${c%%$'\n'*}"
if [[ "$first" != "frame 8" ]]; then
  echo "the scene drew $first, not eight slots" >&2
  exit 1
fi
echo "$first"
