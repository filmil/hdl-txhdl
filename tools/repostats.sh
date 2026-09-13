#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Lines per topic in this repository: every tracked file outside
# third_party, minus the lock files, counted with wc, so comments and
# blank lines count. Run from the repository root.
set -eu
git ls-files |
  grep -v '^third_party/' |
  grep -vE '\.(lock|json)$|^\.bazelversion$' |
  xargs wc -l |
  awk '
    $2 == "total" { next }
    {
      n = $1; p = $2; t = "root config and READMEs"
      if (p ~ /^lib\/src\//)            t = "runtime library"
      else if (p ~ /^lib\/macros\//)    t = "lowering macro"
      else if (p ~ /^lib\/examples\//)  t = "examples"
      else if (p ~ /^lib\//)            t = "lib build files"
      else if (p ~ /^cpu\/vreteno\/src\//)   t = "Vreteno core"
      else if (p ~ /^cpu\/vreteno\/tests\//) t = "Vreteno lockstep test"
      else if (p ~ /^cpu\/vreteno\/board\//) t = "Vreteno board"
      else if (p ~ /^cpu\/vreteno\//)   t = "Vreteno build and synthesis"
      else if (p ~ /^tools\//)          t = "tools"
      else if (p ~ /^docs\/.*\.tex$/)   t = "documents, LaTeX"
      else if (p ~ /^docs\/.*\.md$/)    t = "design notes, Markdown"
      else if (p ~ /^docs\//)           t = "docs build files"
      else if (p ~ /^experiments\//)    t = "Rust probes"
      else if (p ~ /^(spec|filmil)\//)  t = "early specs and proposals"
      else if (p == "proposal.md")      t = "early specs and proposals"
      else if (p ~ /^\.forgejo\//)      t = "CI workflows"
      lines[t] += n; files[t] += 1; all += n; count += 1
      split(p, parts, "/"); base = parts[length(parts)]
      ext = (base ~ /\./) ? base : "none"; sub(/.*\./, "", ext)
      lang[ext] += n
    }
    END {
      for (t in lines) printf "T %7d %5d %s\n", lines[t], files[t], t
      printf "T %7d %5d total\n", all, count
      for (e in lang) printf "L %7d %5d %s\n", lang[e], 0, e
    }' |
  sort -k1,1r -k2,2nr |
  awk '
    $1 == "T" { printf "%-28s %5d %7d\n", substr($0, 17), $3, $2 }
    $1 == "L" { printf "%-28s %13d\n", $4, $2 }'
