/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The conformance suite's run (#999): the machine, a window and a context
 * through EGL as any program takes them, then each group of tests, and
 * the count.
 */
#include <EGL/egl.h>
#include <stdarg.h>
#include <stdio.h>

#include "conform.h"

static int passed, failed;
static EGLDisplay dpy;
static EGLSurface surface;
static EGLContext context;

/* GL's initial state again: the context made not current and current
 * again, which EGL starts anew, as a new context would be. Each group
 * and each of piglit's tests begins with it. */
void conform_fresh(void) {
  eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
  eglMakeCurrent(dpy, surface, surface, context);
}

void conform_pass(const char *group, const char *name) {
  printf("PASS %s.%s\n", group, name);
  passed++;
}

void conform_fail(const char *group, const char *name, const char *fmt, ...) {
  va_list ap;
  printf("FAIL %s.%s: ", group, name);
  va_start(ap, fmt);
  vprintf(fmt, ap);
  va_end(ap);
  printf("\n");
  failed++;
}

int main(void) {
  conform_machine();
  dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
  EGLint major, minor;
  static const EGLint want[] = {EGL_RED_SIZE,     8, EGL_GREEN_SIZE, 8,
                                EGL_BLUE_SIZE,    8, EGL_DEPTH_SIZE, 16,
                                EGL_STENCIL_SIZE, 8, EGL_NONE};
  static const EGLint es1[] = {EGL_CONTEXT_CLIENT_VERSION, 1, EGL_NONE};
  EGLConfig config;
  EGLint n = 0;
  if (!eglInitialize(dpy, &major, &minor) ||
      !eglChooseConfig(dpy, want, &config, 1, &n) || n != 1) {
    printf("FAIL egl.setup: no configuration, error %04x\n", eglGetError());
    return 1;
  }
  surface = eglCreateWindowSurface(dpy, config, 0, 0);
  context = eglCreateContext(dpy, config, EGL_NO_CONTEXT, es1);
  if (surface == EGL_NO_SURFACE || context == EGL_NO_CONTEXT ||
      !eglMakeCurrent(dpy, surface, surface, context)) {
    printf("FAIL egl.setup: no window, error %04x\n", eglGetError());
    return 1;
  }
  conform_state();
  conform_fresh();
  conform_errors();
  conform_fresh();
  conform_render();
  conform_piglit();
  printf("DONE %d passed, %d failed\n", passed, failed);
  return 0;
}
