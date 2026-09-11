#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Bazel's workspace status: the commit every document is stamped with.
# STABLE_ keys are part of the action key, so a stamped output is
# rebuilt when the commit changes and not when only the clock does.
commit=$(git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
  commit="${commit}-dirty"
fi
echo "STABLE_GIT_COMMIT ${commit}"
