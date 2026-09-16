# uberddr3

UberDDR3, an open source DDR3 controller, from
https://github.com/AngeloJacobo/UberDDR3 at commit
`a1258e2eedefa6fba1a425becaded83928c78709`.
The a200t_examples repository runs that commit on the same Alinx
AX7A200 board, which is why it is this one.

Unlike the other directories here, this one holds no upstream files.
The repository is fetched by `git_repository` in `MODULE.bazel`, and
this directory holds only its build file, `uberddr3.BUILD.bazel`.
The reason is the licence: UberDDR3 is GPL-3.0 and this tree is
Apache-2.0, and fetching keeps the two apart in the tree.
A bitstream built with the controller is still subject to the GPL.

The build reads the controller's RTL and the Micron model of a DDR3
chip for simulation, and nothing else of it.
