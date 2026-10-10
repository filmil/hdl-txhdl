#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Runs //tools/datasheet:whole over the generated tables and the
# datasheets, with pdftotext from the pinned poppler tree, found in the
# runfiles by the file it holds.
#
#   tables_whole_test.sh <whole> <ds_tables.tex> <datasheets.pdf>
set -euo pipefail
whole="$1" tex="$2" pdf="$3"
tool=$(find -L "${RUNFILES_DIR:-.}" -path '*/tree/usr/bin/pdftotext' -print -quit 2>/dev/null || true)
[ -n "$tool" ] || { echo "no pinned pdftotext in the runfiles" >&2; exit 1; }
exec "$whole" --pdftotext "$(cd "$(dirname "$tool")" && pwd)/pdftotext" "$tex" "$pdf"
