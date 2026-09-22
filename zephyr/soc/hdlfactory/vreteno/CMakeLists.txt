# SPDX-License-Identifier: Apache-2.0

zephyr_include_directories(.)

# This machine needs no linker script of its own. It has no memory
# outside what the device tree already describes: the boot memory, the
# data memory and the DDR3 the image is linked into. So the common
# RISC-V script is the whole of it, named here rather than included
# from a `linker.ld` beside this file, which is the older spelling and
# which Zephyr deprecates.
set(SOC_LINKER_SCRIPT
    ${ZEPHYR_BASE}/include/zephyr/arch/riscv/common/linker.ld
    CACHE INTERNAL "")
