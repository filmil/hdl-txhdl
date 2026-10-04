# SPDX-License-Identifier: Apache-2.0
"""Genrule commands that stay quiet unless they fail."""

def quiet_cmd(cmd):
    """Wraps a genrule command so that its stderr is printed only on failure.

    Bazel stores an action's output with its result and prints it again on
    every cache hit. A tool that logs as it works, such as vcdcvt and
    sqlite2drawtiming with -logtostderr, then fills every CI log with
    lines from actions that did not run. The wrapped command writes its
    stderr to a temporary file, and prints that file only if the command
    fails, with the command's own exit status.

    Args:
      cmd: the genrule command, with the usual genrule substitutions
        ($(location ...), $@, $$ for a literal $). Its stdout is left alone,
        so a command that writes its output with `> $@` still works.

    Returns:
      The wrapped command, for a genrule's `cmd`.
    """
    return ("_log=$$(mktemp) && ( " + cmd + " ) 2>$$_log" +
            " || { _rc=$$?; cat $$_log >&2; exit $$_rc; }")
