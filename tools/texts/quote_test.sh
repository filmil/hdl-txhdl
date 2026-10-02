#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Every text named reached the test and has something in it, and the
# seven the issue names are all among them (issue 838).
set -euo pipefail
for f in "$@"; do
  [[ -s "$f" ]] || { echo "missing or empty: $f" >&2; exit 1; }
done
for want in spec/language.md spec/merged-draft.md filmil/theses.md \
    filmil/workspace/README.md filmil/workspace/draft-spec.md \
    filmil/workspace/lhdl-proposal.md filmil/workspace/spec.md; do
  printf '%s\n' "$@" | grep -q "$want\$" || {
    echo "not quoted: $want" >&2
    exit 1
  }
done
echo "quoted: $(printf '%s\n' "$@" | sort -u | wc -l) texts"
