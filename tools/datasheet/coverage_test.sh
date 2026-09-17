#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Every component has a datasheet. A component is a unit under
# `#[lower]` in the crates this test is given, or a type a family macro
# writes (`station!`, `router!`, `lite_bridge!`, `plic!`). A datasheet
# says what it covers on a line `% covers: A, B, C` in
# `docs/datasheets/`. The test lists each component nobody covers, and
# each datasheet the document does not include, and fails if there is
# either.
set -euo pipefail

sheets=$(find -L . -path '*docs/datasheets/*.tex' | sort)
doc=$(find -L . -path '*docs/datasheets.tex' | head -1)
if [[ -z "$sheets" || -z "$doc" ]]; then
  echo "no datasheets found in the runfiles" >&2
  exit 1
fi

# What the datasheets cover.
covered=$(grep -h '^% covers:' $sheets | sed 's/^% covers://' \
  | tr ',' '\n' | tr -d ' ' | grep -v '^$' | sort -u)

# The components: the type after `for` in each lowered `impl ... Unit
# for Name`, read across the lines up to its opening brace, and the
# first argument of each family macro.
sources=$(find -L . \( -path '*/lib/parts/src/*' -o -path '*/cpu/vreteno/src/*' \
  -o -path '*/gpu/razboj/src/*' -o -path '*/ddr3/src/*' \
  -o -path '*/pcie/src/*' \) -name '*.rs' | sort)
components=$(
  for f in $sources; do
    awk '
      /^#\[lower\]/ { take = 1; head = ""; next }
      take {
        head = head " " $0
        if ($0 ~ /\{[[:space:]]*$/ || $0 ~ /^[[:space:]]*\{/) {
          if (head ~ /impl/ && head ~ /Unit/) {
            sub(/.*[[:space:]]for[[:space:]]+/, "", head)
            sub(/[^A-Za-z0-9_].*/, "", head)
            print head
          }
          take = 0
        }
        if ($0 ~ /^[[:space:]]*(pub[[:space:]]+)?fn[[:space:]]/) { take = 0 }
      }
      /^[[:space:]]*(station|router|lite_bridge|plic)!\(/ {
        s = $0
        sub(/^[^(]*\(/, "", s)
        sub(/,.*/, "", s)
        print s
      }
    ' "$f"
  done | sort -u
)

missing=$(comm -23 <(echo "$components") <(echo "$covered"))
status=0
if [[ -n "$missing" ]]; then
  echo "components with no datasheet:" >&2
  echo "$missing" | sed 's/^/  /' >&2
  status=1
fi

# Every datasheet is in the document.
for s in $sheets; do
  base=$(basename "$s" .tex)
  if ! grep -q "\\\\input{datasheets/$base}" "$doc"; then
    echo "datasheet not in docs/datasheets.tex: $base" >&2
    status=1
  fi
done

echo "$(echo "$components" | wc -l) components, all covered" \
  | { [[ $status -eq 0 ]] && cat || true; }
exit $status
