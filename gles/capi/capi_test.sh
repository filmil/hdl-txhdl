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
# A clear, the quad's two triangles and the fan's two: an empty frame
# on both sides is not a pass.
if [[ "$(echo "$c" | head -1)" != "frame 5" ]]; then
  echo "the scene drew $(echo "$c" | head -1), not five instructions" >&2
  exit 1
fi
echo "$c" | head -1
