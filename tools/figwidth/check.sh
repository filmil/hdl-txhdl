#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs figwidth as a test: the tool, then the diagram, then the sources,
# as //docs/waveform.bzl passes them.
set -euo pipefail
tool="$1"
timing="$2"
shift 2
exec "$tool" --timing "$timing" "$@"
