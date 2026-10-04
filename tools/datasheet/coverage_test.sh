#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Every component has a datasheet. A component is a unit under
# `#[lower]` in the crates this test is given, or a type a family macro
# writes (`station!`). A datasheet
# says what it covers on a line `% covers: A, B, C` in
# `docs/datasheets/`. The test lists each component nobody covers, and
# each datasheet the document does not include, and fails if there is
# either.
#
# A component is named by its path as well as its type: its crate, from
# the directory it is in, and its module, from its file, so
# `lib/parts/src/hdmi.rs`'s `Raster` is `txhdl_parts::hdmi::Raster`. A
# sheet may cover a component by either name, unless two components
# share the type's name: then only the path says which, and a sheet
# that names the type alone covers neither (issue 1055).
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
# The crate a file is in, by the directory its sources are under, and
# the module the file is, so that a component has a path.
module_of() {
  local f=$1 crate rest
  case "$f" in
    */lib/parts/src/*) crate=txhdl_parts; rest=${f#*/lib/parts/src/} ;;
    */cpu/vreteno/src/*) crate=vreteno32; rest=${f#*/cpu/vreteno/src/} ;;
    */gpu/razboj/src/*) crate=razboj; rest=${f#*/gpu/razboj/src/} ;;
    */ddr3/src/*) crate=ddr3; rest=${f#*/ddr3/src/} ;;
    */pcie/src/*) crate=pcie; rest=${f#*/pcie/src/} ;;
    *) echo "a source outside the known crates: $f" >&2; exit 1 ;;
  esac
  rest=${rest%.rs}
  rest=${rest%/mod}
  [[ "$rest" == lib ]] && rest=""
  echo "$crate${rest:+::${rest//\//::}}"
}

# Each component as its path and its type's name, one to a line.
components=$(
  for f in $sources; do
    m=$(module_of "$f")
    awk -v m="$m" '
      /^#\[lower\]/ { take = 1; head = ""; next }
      take {
        head = head " " $0
        if ($0 ~ /\{[[:space:]]*$/ || $0 ~ /^[[:space:]]*\{/) {
          if (head ~ /impl/ && head ~ /Unit/) {
            sub(/.*[[:space:]]for[[:space:]]+/, "", head)
            sub(/[^A-Za-z0-9_].*/, "", head)
            print m "::" head, head
          }
          take = 0
        }
        if ($0 ~ /^[[:space:]]*(pub[[:space:]]+)?fn[[:space:]]/) { take = 0 }
      }
      /^[[:space:]]*station!\(/ {
        s = $0
        sub(/^[^(]*\(/, "", s)
        sub(/,.*/, "", s)
        print m "::" s, s
      }
    ' "$f"
  done | sort -u
)

# The type names two components share: those only a path covers.
shared=$(echo "$components" | awk '{ print $2 }' | sort | uniq -d)

missing=$(
  echo "$components" | while read -r path name; do
    if grep -qxF "$path" <<<"$covered"; then
      continue
    fi
    if ! grep -qxF "$name" <<<"$shared" && grep -qxF "$name" <<<"$covered"; then
      continue
    fi
    if grep -qxF "$name" <<<"$shared"; then
      echo "$path (another component is also $name: name it by its path)"
    else
      echo "$path"
    fi
  done
)
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
