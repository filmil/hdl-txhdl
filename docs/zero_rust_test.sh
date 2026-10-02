#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The blinky workspace compiles with the Rust the tree compiles with
# (issue 840). The Rust rules read the version only from the root
# module, so the workspace names it again in its own MODULE.bazel, and
# this fails when the two copies part.
#
#   zero_rust_test.sh TREE_MODULE BLINKY_MODULE
set -euo pipefail

# The stable versions in a module's rust.toolchain(...) call, in order.
stable() {
	awk '/^rust\.toolchain\(/{on=1} on{print} on && /^\)/{exit}' "$1" |
		grep -oE '"[0-9]+\.[0-9]+\.[0-9]+"' | tr -d '"'
}

tree="$(stable "$1" | head -1)"
blinky="$(stable "$2" | head -1)"
if [[ -z "$tree" || -z "$blinky" ]]; then
	echo "no stable Rust version found: tree '$tree', blinky '$blinky'" >&2
	exit 1
fi
if [[ "$tree" != "$blinky" ]]; then
	echo "the tree compiles with Rust $tree and the blinky workspace with $blinky" >&2
	exit 1
fi
echo "both compile with Rust $tree"
