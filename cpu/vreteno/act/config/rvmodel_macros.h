# SPDX-License-Identifier: Apache-2.0
# rvmodel_macros.h: the macros the architectural tests leave to the
# device under test, for the runner in //cpu/vreteno/act.
#
# The runner speaks HTIF through `tohost`, as the Sail reference
# model does, so one set of macros serves both: the signature run on
# Sail and the final run on the core's simulation.
#
# Derived from config/sail/sail-rv32-max/rvmodel_macros.h in
# riscv-arch-test (BSD-3-Clause), with the interrupt and timer macros
# removed: the runner has no timer and no interrupt generator.

#ifndef _RVMODEL_MACROS_H
#define _RVMODEL_MACROS_H

#define RVMODEL_DATA_SECTION \
        .pushsection .tohost,"aw",@progbits;                \
        .balign 8; .global tohost; tohost: .dword 0;         \
        .balign 8; .global fromhost; fromhost: .dword 0;     \
        .popsection

#define STANDARD_SM_SUPPORTED

##### TERMINATION #####

# A pass writes 1 to `tohost`, a fail writes 3. The write to the upper
# half completes the command.
#define RVMODEL_HALT_PASS  \
  li x1, 1                ;\
  la t0, tohost           ;\
  write_tohost_pass:      ;\
    sw x1, 0(t0)          ;\
    sw x0, 4(t0)          ;\
    j write_tohost_pass   ;\

#define RVMODEL_HALT_FAIL \
  li x1, 3                ;\
  la t0, tohost           ;\
  write_tohost_fail:      ;\
    sw x1, 0(t0)          ;\
    sw x0, 4(t0)          ;\
    j write_tohost_fail   ;\

##### IO #####

# One character at a time, through HTIF device 1 (the console),
# command 1 (write).
#define RVMODEL_IO_WRITE_STR(_R1, _R2, _R3, _STR_PTR) \
1:                         ;   \
  lbu _R1, 0(_STR_PTR)     ;   \
  beqz _R1, 3f             ;   \
  la _R2, tohost           ;   \
  sw _R1, 0(_R2)           ;   \
  li _R1, 0x01010000       ;   \
  sw _R1, 4(_R2)           ;   \
  addi _STR_PTR, _STR_PTR, 1 ; \
  j 1b                     ;   \
3:

##### Interrupts #####

# The runner raises no interrupt, so no test that needs one is
# selected. The latency is still required by the framework.
#define RVMODEL_INTERRUPT_LATENCY 1
#define RVMODEL_TIMER_INT_SOON_DELAY 5000

#endif // _RVMODEL_MACROS_H
