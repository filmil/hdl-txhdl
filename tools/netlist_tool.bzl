# SPDX-License-Identifier: Apache-2.0
"""The rustc flags of a tool that writes netlists (#1792).

A genrule's tool is built in the exec configuration, which is `-c opt`,
so at opt-level 3. A tool that calls a unit's `lowered()` compiles the
netlist generator `#[lower]` writes, one function per unit, and LLVM
spends minutes and gigabytes optimising code that runs once (#1807).
Measured on main 30f59919 under `--config=ci`: `datasheet` took 2079 s
and 3.83 GiB to compile and 0.1 s to run, `board_netlist` 1447 s and
3.60 GiB, Razboj's trace tools 647 to 1339 s and 2.68 GiB each.

At opt-level 0 the generator compiles in seconds. A tool that also
runs a simulation runs it slower, so a tool takes these flags only
where its compile and its run together come out ahead.
"""

NETLIST_TOOL_RUSTC_FLAGS = ["-Copt-level=0"]
