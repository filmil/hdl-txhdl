/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The parts of piglit's utility library that the four ported ES 1 tests
 * use (#999), written for the conformance suite rather than taken from
 * piglit: each test defines PIGLIT_PORT before it includes this, and its
 * piglit_init, piglit_display and configuration are renamed by it, so that
 * the four link into one program; piglit_report_result returns to the
 * suite's runner, in piglit.c, rather than ending the program.
 */
#ifndef PIGLIT_UTIL_GL_H
#define PIGLIT_UTIL_GL_H

#include <GLES/gl.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#define ARRAY_SIZE(a) (sizeof(a) / sizeof((a)[0]))

/* The ES 1 build of each test, as piglit's own ES 1 build defines it. */
#define PIGLIT_USE_OPENGL_ES1 1

/* Names the tests use that ES 1.1's gl.h does not hold: the point sprite's
 * origins, which only the desktop path of its test reads, and
 * OES_matrix_get's, a core addition in ES 1.1, from glext.h. */
#ifndef GL_UPPER_LEFT
#define GL_UPPER_LEFT 0x8CA2
#define GL_LOWER_LEFT 0x8CA1
#endif
#ifndef GL_MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES
#define GL_MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES 0x898D
#define GL_PROJECTION_MATRIX_FLOAT_AS_INT_BITS_OES 0x898E
#define GL_TEXTURE_MATRIX_FLOAT_AS_INT_BITS_OES 0x898F
#endif

enum piglit_result { PIGLIT_PASS, PIGLIT_FAIL, PIGLIT_SKIP, PIGLIT_WARN };

/* The configuration's fields the four set; the window is the suite's. */
struct piglit_gl_test_config {
  int supports_gl_es_version;
  int supports_gl_compat_version;
  int window_visual;
  int khr_no_error_support;
  int window_width;
  int window_height;
};
#define PIGLIT_GL_VISUAL_RGB 1
#define PIGLIT_GL_VISUAL_RGBA 2
#define PIGLIT_GL_VISUAL_DOUBLE 4
#define PIGLIT_NO_ERRORS 1

/* Each test's names, by its PIGLIT_PORT. */
#define PIGLIT_CAT2(a, b) a##b
#define PIGLIT_CAT(a, b) PIGLIT_CAT2(a, b)
#define piglit_init PIGLIT_CAT(PIGLIT_PORT, _init)
#define piglit_display PIGLIT_CAT(PIGLIT_PORT, _display)
#define PIGLIT_GL_TEST_CONFIG_BEGIN                                            \
  void PIGLIT_CAT(PIGLIT_PORT, _config)(struct piglit_gl_test_config *out) {   \
    struct piglit_gl_test_config config;                                       \
    memset(&config, 0, sizeof config);
#define PIGLIT_GL_TEST_CONFIG_END                                              \
  *out = config;                                                               \
  }

void piglit_init(int argc, char **argv);
enum piglit_result piglit_display(void);

/* The window, and that no one watches. */
extern int piglit_width, piglit_height;
extern bool piglit_automatic;

/* Ends the test with `result`, back in the runner. */
void piglit_report_result(enum piglit_result result);
/* Skips the test unless the EXTENSIONS string holds `name`. */
void piglit_require_extension(const char *name);
/* Whether GetError gives `expected`, saying so when it does not. */
bool piglit_check_gl_error(GLenum expected);
const char *piglit_get_gl_enum_name(GLenum e);
const char *piglit_get_gl_error_name(GLenum e);
/* Whether the pixels read back hold the colour, within piglit's tolerance
 * of a channel of eight bits. */
bool piglit_probe_rect_rgba(int x, int y, int w, int h, const float *expected);
bool piglit_probe_pixel_rgb(int x, int y, const float *expected);
void piglit_present_results(void);
/* An orthographic projection of the window, its origin bottom left. */
void piglit_ortho_projection(int w, int h, GLboolean push);
/* A texture of `w` by `h` texels, checkered in squares of the two
 * colours, its filters nearest, bound; its name. */
GLuint piglit_checkerboard_texture(GLuint tex, unsigned level, unsigned w,
                                   unsigned h, unsigned horiz, unsigned vert,
                                   const float *black, const float *white);

#endif
