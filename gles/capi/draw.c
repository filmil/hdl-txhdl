/* SPDX-License-Identifier: Apache-2.0 */
/*
 * A scene drawn through the GL library's C entry points, as any GL ES
 * 1.1 program draws (issue 1224): Khronos's <GLES/gl.h>, a projection
 * and a modelview, a light, the depth test, blending, client arrays of three types with a stride,
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
extern void gles_buffer_room(unsigned char *mem, size_t bytes);

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
  glEnable(GL_BLEND);
  glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
  glEnable(GL_ALPHA_TEST);
  glAlphaFuncx(GL_GREATER, ONE / 4);
  glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
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
  // The queries (#1484), of state set above: the library's, and the
  // client arrays' the C API keeps.
  GLint v[4];
  glGetIntegerv(GL_VIEWPORT, v);
  printf("viewport %d %d %d %d\n", v[0], v[1], v[2], v[3]);
  glGetIntegerv(GL_DEPTH_FUNC, v);
  printf("depth func %04x\n", v[0]);
  GLboolean mask[4];
  glGetBooleanv(GL_COLOR_WRITEMASK, mask);
  printf("colour mask %d%d%d%d\n", mask[0], mask[1], mask[2], mask[3]);
  GLfixed range[2];
  glGetFixedv(GL_DEPTH_RANGE, range);
  printf("depth range %d %d\n", range[0], range[1]);
  glGetIntegerv(GL_VERTEX_ARRAY_SIZE, v);
  glGetIntegerv(GL_VERTEX_ARRAY_TYPE, v + 1);
  glGetIntegerv(GL_VERTEX_ARRAY_STRIDE, v + 2);
  printf("vertex array %d %04x %d\n", v[0], v[1], v[2]);
  void *at = 0;
  glGetPointerv(GL_VERTEX_ARRAY_POINTER, &at);
  printf("vertex pointer %d\n", at == (void *)fan);
  /* Fog's state (#998), through both its entry points. */
  glFogx(GL_FOG_MODE, GL_LINEAR);
  glFogx(GL_FOG_END, 8 * ONE);
  static const GLfixed fog[4] = {ONE / 2, 2 * ONE, -ONE, ONE};
  glFogxv(GL_FOG_COLOR, fog);
  static const GLfixed density = ONE / 4;
  glFogxv(GL_FOG_DENSITY, &density);
  GLfixed got[4];
  glGetFixedv(GL_FOG_COLOR, got);
  printf("fog colour %d %d %d %d\n", got[0], got[1], got[2], got[3]);
  glGetFixedv(GL_FOG_END, got);
  glGetFixedv(GL_FOG_DENSITY, got + 1);
  glGetIntegerv(GL_FOG_MODE, v);
  printf("fog %04x end %d density %d\n", v[0], got[0], got[1]);


  /* Buffer objects (#1488): a new context with room for their stores,
     and the quad drawn from them, its positions and, at an offset into
     the same buffer, its colours, and its indices from an element
     buffer. */
  static unsigned char room[1024];
  gles_make_current(frame, CAP, 64, 48);
  gles_buffer_room(room, sizeof room);
  GLuint names[2];
  glGenBuffers(2, names);
  glBindBuffer(GL_ARRAY_BUFFER, names[0]);
  glBufferData(GL_ARRAY_BUFFER, sizeof quad + sizeof colours, 0,
               GL_STATIC_DRAW);
  glBufferSubData(GL_ARRAY_BUFFER, 0, sizeof quad, quad);
  glBufferSubData(GL_ARRAY_BUFFER, sizeof quad, sizeof colours, colours);
  glEnableClientState(GL_VERTEX_ARRAY);
  glEnableClientState(GL_COLOR_ARRAY);
  glVertexPointer(3, GL_SHORT, sizeof quad[0], (const void *)0);
  glColorPointer(4, GL_UNSIGNED_BYTE, 0, (const void *)sizeof quad);
  glBindBuffer(GL_ARRAY_BUFFER, 0);
  glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, names[1]);
  glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof indices, indices,
               GL_STATIC_DRAW);
  glScalex(ONE / 2, ONE / 2, ONE);
  glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, (const void *)0);
  glBindBuffer(GL_ARRAY_BUFFER, names[0]);
  GLint size = 0;
  glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &size);
  printf("buffers %u %u size %d is %d\n", names[0], names[1], size,
         glIsBuffer(names[0]));
  GLint bound[2];
  glGetIntegerv(GL_ELEMENT_ARRAY_BUFFER_BINDING, bound);
  glGetIntegerv(GL_VERTEX_ARRAY_BUFFER_BINDING, bound + 1);
  printf("bindings %d %d\n", bound[0], bound[1]);
  size_t m = gles_frame_len();
  printf("buffer frame %zu\n", m);
  for (size_t i = 0; i < m * 16; i++) {
    printf("%08x\n", frame[i]);
  }
  printf("error %04x\n", glGetError());
  return 0;
}
