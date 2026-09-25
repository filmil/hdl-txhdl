#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# The full suite, `bazel test //...`, and where each result came from.
# Run from the repository root; arguments after the flags go to Bazel.
#
#   tools/suite.sh            # the suite, then the account
#   tools/suite.sh --fresh    # every test run here, none from a cache
#
# A test's result is kept by its inputs, by content, so "cached" is
# only as old as those inputs, and it can come from three places: run
# here, this output base's own cache, or the disk cache every
# worktree on this host and CI share (/data/cache/ci.bazelrc, which
# .bazelrc imports). A suite that finishes in seconds after a rebase
# is usually the third: somebody already tested these exact inputs.
# This says which it was, so a green result is read for what it is
# rather than guessed at from its speed (issue 618).
set -euo pipefail

fresh=()
if [[ "${1:-}" == "--fresh" ]]; then
  fresh=(--nocache_test_results)
  shift
fi

log=$(mktemp -t suite-bep.XXXXXX.json)
trap 'rm -f "$log"' EXIT

status=0
bazel test //... "${fresh[@]}" --build_event_json_file="$log" "$@" ||
  status=$?

# One event per line. A test result's own line names it in its id and
# says whether it was cached in this output base (`cachedLocally`), or
# was taken from the disk cache (the `disk cache hit` strategy).
here=0 local=0 shared=0
while IFS= read -r line; do
  case "$line" in
    *'"id":{"testResult"'*) ;;
    *) continue ;;
  esac
  if [[ "$line" == *'"cachedLocally":true'* ]]; then
    local=$((local + 1))
  elif [[ "$line" == *'"strategy":"disk cache hit"'* ||
    "$line" == *'"cachedRemotely":true'* ]]; then
    shared=$((shared + 1))
  else
    here=$((here + 1))
  fi
done <"$log"

tree="clean"
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
  tree="with uncommitted changes"
fi
echo
echo "suite over $(git rev-parse --short HEAD), $tree, in $(pwd)"
echo "  run here:                       $here"
echo "  from this output base's cache:  $local"
echo "  from the shared disk cache:     $shared"
if [[ ${#fresh[@]} -eq 0 && $here -eq 0 ]]; then
  echo "  No test ran here: every result is one these exact inputs"
  echo "  produced before. Run with --fresh to run them all here."
fi
exit "$status"
