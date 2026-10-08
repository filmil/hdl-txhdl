// SPDX-License-Identifier: Apache-2.0
// rvtest_config.h: what the Vreteno core implements, in the macros the
// architectural tests read.
//
// The ACT4 framework writes this file from a UDB description of the
// core, with Ruby tools this build does not use. It is written here by
// hand instead, from the core's own source (`cpu/vreteno/src/isa.rs`
// and `cpu/vreteno/src/core.rs`), and it must agree with `sail.json`
// beside it and with the lists in `//cpu/vreteno/act:BUILD.bazel`,
// which select the tests. It came from vreteno-conformance (issue
// 1442).

#ifndef RVTEST_CONFIG_H
#define RVTEST_CONFIG_H

// RV32, misa = I, M, A, C, S, U.
#define UDB_MXLEN 32
#define I_SUPPORTED
#define M_SUPPORTED
#define ZMMUL_SUPPORTED
#define A_SUPPORTED
#define ZAAMO_SUPPORTED
#define ZALRSC_SUPPORTED
#define C_SUPPORTED
#define ZCA_SUPPORTED
#define ZICSR_SUPPORTED
#define ZIFENCEI_SUPPORTED
#define ZICNTR_SUPPORTED

// Machine, supervisor and user modes, and Sv32, described as
// privileged version 1.11, as vreteno-conformance describes it, which
// keeps the Svade suite out (issue 1348).
#define SM_SUPPORTED
#define SM1P11P0_SUPPORTED
#define S_SUPPORTED
#define S1P11P0_SUPPORTED
#define U_SUPPORTED
#define SV32_SUPPORTED

// The time CSR reads the core's `time` input; no emulation is needed.
#define UDB_TIME_CSR_IMPLEMENTED

// mtvec and stvec are direct only, four-byte aligned.
#define UDB_MTVEC_BASE_ALIGNMENT_DIRECT 4
#define UDB_STVEC_BASE_ALIGNMENT_DIRECT 4

// No physical memory protection. mcountinhibit and the hardware
// performance counters are there and read as zero (issue 1460), so
// no Zihpm.
#define UDB_NUM_PMP_ENTRIES 0
#define UDB_NUM_USABLE_PMP_ENTRIES 0

#endif // RVTEST_CONFIG_H
