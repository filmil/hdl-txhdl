/* SPDX-License-Identifier: Apache-2.0 */
/*
 * A Zephyr program on Vreteno drawing through EGL and GL ES 1.1, which
 * Razboj rasterises (issue 996): a fan of six triangles, lit, turning a
 * little each frame, double buffered and swapped on the vertical
 * blanking, so that no frame is shown half drawn.
 *
 * It says `gles egl 1.4` once EGL is up, then every sixty frames the
 * frame count and the cycles the last frame took, from the clear to
 * the end of the swap.
 */
#include <EGL/egl.h>
#include <GLES/gl.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

/* Not EGL: the board's machine for EGL, before eglInitialize. */
extern void egl_vreteno_install(void);

#define ONE 65536

/* The fan: its centre and seven points round it, the last the first. */
static const GLfixed fan[8][3] = {
    {0, 0, ONE / 4},
    {ONE / 2, 0, 0},
    {ONE / 4, 7 * ONE / 16, 0},
    {-ONE / 4, 7 * ONE / 16, 0},
    {-ONE / 2, 0, 0},
    {-ONE / 4, -7 * ONE / 16, 0},
    {ONE / 4, -7 * ONE / 16, 0},
    {ONE / 2, 0, 0},
};
/* A normal at each, leaning out from the centre. */
static const GLbyte normals[8][3] = {
    {0, 0, 127},  {90, 0, 90},  {45, 78, 90},  {-45, 78, 90},
    {-90, 0, 90}, {-45, -78, 90}, {45, -78, 90}, {90, 0, 90},
};

static uint32_t cycles(void) {
  uint32_t c;
  __asm__ volatile("csrr %0, mcycle" : "=r"(c));
  return c;
}

int main(void) {
  egl_vreteno_install();
  EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
  EGLint major = 0, minor = 0;
  if (!eglInitialize(dpy, &major, &minor)) {
    printk("gles egl failed %x\n", eglGetError());
    return 0;
  }
  static const EGLint want[] = {EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8,
                                EGL_BLUE_SIZE, 8, EGL_SURFACE_TYPE,
                                EGL_WINDOW_BIT, EGL_NONE};
  EGLConfig config;
  EGLint n = 0;
  eglChooseConfig(dpy, want, &config, 1, &n);
  EGLSurface surface = eglCreateWindowSurface(dpy, config, 0, 0);
  static const EGLint es1[] = {EGL_CONTEXT_CLIENT_VERSION, 1, EGL_NONE};
  EGLContext ctx = eglCreateContext(dpy, config, EGL_NO_CONTEXT, es1);
  if (n != 1 || surface == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT ||
      !eglMakeCurrent(dpy, surface, surface, ctx)) {
    printk("gles egl setup failed %x\n", eglGetError());
    return 0;
  }
  printk("gles egl %d.%d\n", major, minor);

  /* A projection whose window is twice as wide as it is high, less a
   * quarter, as 640 by 480 is, and a light at the eye. */
  glMatrixMode(GL_PROJECTION);
  glLoadIdentity();
  glFrustumx(-ONE / 2, ONE / 2, -3 * ONE / 8, 3 * ONE / 8, ONE, 8 * ONE);
  glMatrixMode(GL_MODELVIEW);
  glLoadIdentity();
  static const GLfixed light[4] = {0, 0, ONE, 0};
  glLightxv(GL_LIGHT0, GL_POSITION, light);
  glEnable(GL_LIGHTING);
  glEnable(GL_LIGHT0);
  glEnable(GL_NORMALIZE);
  static const GLfixed gold[4] = {ONE, 3 * ONE / 4, ONE / 4, ONE};
  glMaterialxv(GL_FRONT_AND_BACK, GL_AMBIENT_AND_DIFFUSE, gold);
  glEnableClientState(GL_VERTEX_ARRAY);
  glEnableClientState(GL_NORMAL_ARRAY);
  glVertexPointer(3, GL_FIXED, 0, fan);
  glNormalPointer(GL_BYTE, 0, normals);
  glClearColorx(0, 0, ONE / 8, ONE);

  for (uint32_t frame = 0;; frame++) {
    uint32_t start = cycles();
    glClear(GL_COLOR_BUFFER_BIT);
    glPushMatrix();
    glTranslatex(0, 0, -3 * ONE);
    glRotatex((GLfixed)(frame % 360) * ONE, ONE / 3, ONE, 0);
    glDrawArrays(GL_TRIANGLE_FAN, 0, 8);
    glPopMatrix();
    eglSwapBuffers(dpy, surface);
    uint32_t took = cycles() - start;
    if (frame % 60 == 0) {
      printk("gles frame %u cycles %u\n", frame, took);
    }
  }
  return 0;
}
