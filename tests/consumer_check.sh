#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Whether TxHDL's programs for the core build from a workspace that
# depends on TxHDL (issue 1001): runs a Bazel of its own in
# tests/consumer, whose MODULE.bazel is what such a workspace writes,
# and analyses the targets that failed there. It fetches what TxHDL
# fetches, into an output base of its own, so it is run by hand:
#
#   bazel run //tests:consumer_check            # analysis only
#   bazel run //tests:consumer_check -- --build # and the build
#
# Any other argument goes to that Bazel as it is, such as a shared
# `--repository_cache=DIR`, so that it fetches nothing it has already.
set -euo pipefail

cd "${BUILD_WORKSPACE_DIRECTORY:?run me with bazel run}/tests/consumer"
mode=(--nobuild)
flags=()
for a in "$@"; do
	if [[ "$a" == "--build" ]]; then
		mode=()
	else
		flags+=("$a")
	fi
done
targets=(
	@txhdl//zephyr:hello_world
	@txhdl//soc:demo
	@txhdl//docs:soc_scene
)
bazel build "${mode[@]}" "${flags[@]}" "${targets[@]}"
echo "consumer_check: ${targets[*]} $([[ ${#mode[@]} -gt 0 ]] && echo analyse || echo build) from a dependent workspace"
