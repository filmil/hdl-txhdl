/* SPDX-License-Identifier: Apache-2.0 */
/*
 * A scene drawn through the GL library's C entry points, as any GL ES
 * 1.1 program draws (issue 1224): Khronos's <GLES/gl.h>, a projection
 * and a modelview, a light, the depth test, client arrays of three types with a stride,
 * indices, and the current colour. It prints the frame's instructions,
 * a word at a time in hex, which capi_test holds to the same scene drawn
 * through the Rust API by draw_ref.rs; and then what an entry point the
 * library does not implement, and glGetString, say.
 */
#include <GLES/gl.h>
#include <stdio.h>
#include <string.h>

/* Not GL: what EGL will call, issue 996. */
extern void gles_make_current(unsigned int *frame, size_t capacity,
                              unsigned int width, unsigned int height);
extern size_t gles_frame_len(void);

#define ONE 65536
#define CAP 64

/* A quad: positions as shorts, three to a vertex and one of padding. */
static const GLshort quad[4][4] = {
    {-1, -1, 0, 0}, {1, -1, 0, 0}, {1, 1, 0, 0}, {-1, 1, 0, 0}};
/* Its normals as bytes, leaning the corners outward. */
static const GLbyte normals[4][3] = {
    {-40, -40, 100}, {40, -40, 100}, {40, 40, 100}, {-40, 40, 100}};
/* Its colours as bytes. */
static const GLubyte colours[4][4] = {
    {255, 0, 0, 255}, {0, 255, 0, 255}, {0, 0, 255, 255}, {255, 255, 0, 255}};
static const GLubyte indices[6] = {0, 1, 2, 0, 2, 3};
/* A fan in 16.16, two components, from its second vertex. */
static const GLfixed fan[5][2] = {{99 * ONE, 99 * ONE},
                                  {0, 0},
                                  {ONE / 2, 0},
                                  {ONE / 2, ONE / 2},
                                  {0, ONE / 2}};

int main(void) {
  static unsigned int frame[CAP * 16];
  gles_make_current(frame, CAP, 64, 48);

  glClearColorx(0, 0, ONE / 4, ONE);
  glClearDepthx(ONE / 2);
  glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
  glMatrixMode(GL_PROJECTION);
  glLoadIdentity();
  glFrustumx(-ONE, ONE, -3 * ONE / 4, 3 * ONE / 4, ONE, 10 * ONE);
  glMatrixMode(GL_MODELVIEW);
  glLoadIdentity();
  static const GLfixed light[4] = {0, 0, ONE, 0};
  glLightxv(GL_LIGHT0, GL_POSITION, light);
  glEnable(GL_LIGHTING);
  glEnable(GL_LIGHT0);
  glEnable(GL_COLOR_MATERIAL);
  glEnable(GL_NORMALIZE);
  glTranslatex(0, 0, -3 * ONE);
  glRotatex(30 * ONE, 0, ONE, 0);

  glEnableClientState(GL_VERTEX_ARRAY);
  glEnableClientState(GL_NORMAL_ARRAY);
  glEnableClientState(GL_COLOR_ARRAY);
  glVertexPointer(3, GL_SHORT, sizeof quad[0], quad);
  glNormalPointer(GL_BYTE, 0, normals);
  glColorPointer(4, GL_UNSIGNED_BYTE, 0, colours);
  glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, indices);

  glDisableClientState(GL_NORMAL_ARRAY);
  glDisableClientState(GL_COLOR_ARRAY);
  glDisable(GL_LIGHTING);
  glShadeModel(GL_FLAT);
  glColor4x(ONE, ONE / 2, 0, ONE);
  glEnable(GL_DEPTH_TEST);
  glDepthFunc(GL_LEQUAL);
  glDepthMask(GL_FALSE);
  glDepthRangex(ONE / 4, 3 * ONE / 4);
  glVertexPointer(2, GL_FIXED, 0, fan);
  glDrawArrays(GL_TRIANGLE_FAN, 1, 4);

  size_t n = gles_frame_len();
  printf("frame %zu\n", n);
  for (size_t i = 0; i < n * 16; i++) {
    printf("%08x\n", frame[i]);
  }
  printf("error %04x\n", glGetError());
  glAlphaFunc(GL_LESS, 0.5f);
  printf("unimplemented %04x\n", glGetError());
  printf("version %s\n", (const char *)glGetString(GL_VERSION));
  return 0;
}
