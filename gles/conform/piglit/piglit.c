/* SPDX-License-Identifier: Apache-2.0 */
/*
 * piglit's utility library as the four ported tests use it, and their
 * runner (#999): each test's init, then its display unless init reported
 * already, each result one of the suite's lines. A skip is reported as a
 * failure, since each test asks only for what ES 1.1 requires.
 */
#include <setjmp.h>
#include <stdarg.h>

#include "../conform.h"
#define PIGLIT_PORT runner
#include "piglit-util-gl.h"

int piglit_width = CONFORM_WIDTH, piglit_height = CONFORM_HEIGHT;
bool piglit_automatic = true;

static jmp_buf back;
static const char *why;

void piglit_report_result(enum piglit_result result) {
  longjmp(back, (int)result + 1);
}

void piglit_require_extension(const char *name) {
  const char *s = (const char *)glGetString(GL_EXTENSIONS);
  size_t n = strlen(name);
  for (const char *p = s; p && *p;) {
    const char *end = strchr(p, ' ');
    size_t len = end ? (size_t)(end - p) : strlen(p);
    if (len == n && strncmp(p, name, n) == 0)
      return;
    p = end ? end + 1 : p + len;
  }
  why = name;
  piglit_report_result(PIGLIT_SKIP);
}

static char names[2][16];
static const char *hex(int k, GLenum e) {
  snprintf(names[k], sizeof names[k], "0x%04x", e);
  return names[k];
}
const char *piglit_get_gl_enum_name(GLenum e) { return hex(0, e); }
const char *piglit_get_gl_error_name(GLenum e) { return hex(1, e); }

bool piglit_check_gl_error(GLenum expected) {
  GLenum e = glGetError();
  if (e == expected)
    return true;
  printf("# Unexpected GL error: 0x%04x, expected 0x%04x\n", e, expected);
  return false;
}

/* piglit's tolerance for a channel of eight bits. */
#define TOLERANCE (1.0f / 255 * 1.5f)

static bool probe(int x, int y, int w, int h, const float *expected, int n) {
  static GLubyte p[CONFORM_WIDTH * CONFORM_HEIGHT * 4];
  glReadPixels(x, y, w, h, GL_RGBA, GL_UNSIGNED_BYTE, p);
  for (int j = 0; j < h; j++)
    for (int i = 0; i < w; i++)
      for (int c = 0; c < n; c++) {
        float got = p[(j * w + i) * 4 + c] / 255.0f;
        float d = got - expected[c];
        if (d > TOLERANCE || d < -TOLERANCE) {
          printf("# Probe at (%d,%d) channel %d: %f, expected %f\n", x + i,
                 y + j, c, got, expected[c]);
          return false;
        }
      }
  return true;
}

bool piglit_probe_rect_rgba(int x, int y, int w, int h,
                            const float *expected) {
  return probe(x, y, w, h, expected, 4);
}

bool piglit_probe_pixel_rgb(int x, int y, const float *expected) {
  return probe(x, y, 1, 1, expected, 3);
}

void piglit_present_results(void) {}

void piglit_ortho_projection(int w, int h, GLboolean push) {
  (void)push;
  glMatrixMode(GL_PROJECTION);
  glLoadIdentity();
  glOrthox(0, w * 65536, 0, h * 65536, -65536, 65536);
  glMatrixMode(GL_MODELVIEW);
  glLoadIdentity();
}

GLuint piglit_checkerboard_texture(GLuint tex, unsigned level, unsigned w,
                                   unsigned h, unsigned horiz, unsigned vert,
                                   const float *black, const float *white) {
  static GLubyte texels[64 * 64 * 4];
  if (!tex)
    glGenTextures(1, &tex);
  glBindTexture(GL_TEXTURE_2D, tex);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
  for (unsigned j = 0; j < h; j++)
    for (unsigned i = 0; i < w; i++) {
      const float *c = ((i / horiz + j / vert) & 1) ? black : white;
      for (int k = 0; k < 4; k++)
        texels[(j * w + i) * 4 + k] = (GLubyte)(c[k] * 255 + 0.5f);
    }
  glTexImage2D(GL_TEXTURE_2D, (GLint)level, GL_RGBA, (GLsizei)w, (GLsizei)h,
               0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
  return tex;
}

/* The four, as their PIGLIT_PORT names them. */
#define PORT(name)                                                             \
  void name##_init(int argc, char **argv);                                     \
  enum piglit_result name##_display(void);
PORT(fixed_point)
PORT(matrix_get)
PORT(paletted)
PORT(point_sprite)

static void run(const char *name, void (*init)(int, char **),
                enum piglit_result (*display)(void)) {
  why = 0;
  conform_fresh();
  volatile int jumped = setjmp(back);
  enum piglit_result r;
  if (jumped) {
    r = (enum piglit_result)(jumped - 1);
  } else {
    init(0, 0);
    r = display();
  }
  while (glGetError() != GL_NO_ERROR)
    ;
  if (r == PIGLIT_PASS)
    conform_pass("piglit", name);
  else if (r == PIGLIT_SKIP)
    conform_fail("piglit", name, "skipped: %s missing", why ? why : "?");
  else
    conform_fail("piglit", name, "piglit says it fails");
}

void conform_piglit(void) {
  run("oes_fixed_point-attribute-arrays", fixed_point_init,
      fixed_point_display);
  run("oes_matrix_get-api", matrix_get_init, matrix_get_display);
  run("oes_compressed_paletted_texture-api", paletted_init, paletted_display);
  run("arb_point_sprite-checkerboard_gles1", point_sprite_init,
      point_sprite_display);
}
