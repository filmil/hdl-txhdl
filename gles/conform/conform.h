/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The ES 1.1 conformance suite's framework (#999): our own tests of the
 * GL ES 1.1 specification, Common-Lite profile, version 1.1.12, as the
 * issue's plan has them. Each test prints one line, `PASS <name>` or
 * `FAIL <name>: <why>`, and the run ends with a count. The same programs
 * run on the host, against Razboj's model, and on the board.
 */
#ifndef CONFORM_H
#define CONFORM_H

#include <GLES/gl.h>

/* The window EGL gives: 640 by 480. */
#define CONFORM_WIDTH 640
#define CONFORM_HEIGHT 480

/* One in 16.16. */
#define CONFORM_ONE 65536

/* A test's result, in the group `group`. */
void conform_pass(const char *group, const char *name);
void conform_fail(const char *group, const char *name, const char *fmt, ...);

/* Installs the machine: Razboj's model on the host, the board's on the
 * board. */
extern void conform_machine(void);

/* The groups. */
void conform_state(void);
void conform_errors(void);

#endif
