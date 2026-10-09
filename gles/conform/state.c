/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The state tables (#999): every state variable of GL ES 1.1's section
 * 6.2, Tables 6.3 to 6.24 of the Common-Lite specification, version
 * 1.1.12, with the initial value the table gives, read through the
 * command the table names, GetFixedv where it names GetFloatv, as the
 * Common-Lite profile has it. A variable read with IsEnabled, GetIntegerv,
 * GetBooleanv or GetFixedv is read through the other three as well, and
 * held to section 6.1.2's conversions:
 *  - a boolean is 1 or 0, and 1.0 or 0.0 in fixed point;
 *  - an integer is scaled by 2^16 to fixed point, and an enumeration is
 *    not;
 *  - a fixed-point value rounds to the nearest integer, except a colour
 *    component, a depth range or depth clear value, or a normal, which
 *    maps [-1, 1] onto the integers' range;
 *  - and any of them is FALSE as a boolean only when it is zero.
 * An implementation-dependent value is held to the table's minimum. The
 * matrices as integer bits are OES_matrix_get's, a core addition by Table
 * C.2, and so are checked, as is the EXTENSIONS string for the four
 * required profile extensions.
 */
#include <stdio.h>
#include <string.h>

#include "conform.h"

#define ONE CONFORM_ONE

/* OES_matrix_get's names, a core addition in ES 1.1, from glext.h, which
 * the tree does not fetch. */
#ifndef GL_MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES
#define GL_MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES 0x898D
#define GL_PROJECTION_MATRIX_FLOAT_AS_INT_BITS_OES 0x898E
#define GL_TEXTURE_MATRIX_FLOAT_AS_INT_BITS_OES 0x898F
#endif

/* What a row is read as, and how it converts: IsEnabled, GetBooleanv,
 * GetIntegerv of a number or of an enumeration, GetFixedv of a number or
 * of a unit value (a colour, a depth, a normal). */
enum kind { ENABLED, BOOLEAN, NUMBER, ENUMERATION, FIXED, UNIT };

/* How a row's values are held: equal, at least, a range (the first at
 * most and the second at least), or a mask whose low eight bits are
 * ones. */
enum hold { EQ, AT_LEAST, RANGE, ONES };

struct row {
  const char *name;
  GLenum pname;
  enum kind kind;
  int n;
  enum hold hold;
  long long want[16];
};

#define IDENTITY \
  { ONE, 0, 0, 0, 0, ONE, 0, 0, 0, 0, ONE, 0, 0, 0, 0, ONE }

static const struct row rows[] = {
    /* Table 6.3, current values. */
    {"CURRENT_COLOR", GL_CURRENT_COLOR, UNIT, 4, EQ, {ONE, ONE, ONE, ONE}},
    {"CURRENT_TEXTURE_COORDS", GL_CURRENT_TEXTURE_COORDS, FIXED, 4, EQ,
     {0, 0, 0, ONE}},
    {"CURRENT_NORMAL", GL_CURRENT_NORMAL, UNIT, 3, EQ, {0, 0, ONE}},
    /* Tables 6.4 and 6.5, vertex arrays. The tables give each type as
     * FLOAT, which is the Common profile's; section 2.8 makes them FIXED
     * in the Common-Lite profile. */
    {"CLIENT_ACTIVE_TEXTURE", GL_CLIENT_ACTIVE_TEXTURE, ENUMERATION, 1, EQ,
     {GL_TEXTURE0}},
    {"VERTEX_ARRAY", GL_VERTEX_ARRAY, ENABLED, 1, EQ, {0}},
    {"VERTEX_ARRAY_SIZE", GL_VERTEX_ARRAY_SIZE, NUMBER, 1, EQ, {4}},
    {"VERTEX_ARRAY_TYPE", GL_VERTEX_ARRAY_TYPE, ENUMERATION, 1, EQ,
     {GL_FIXED}},
    {"VERTEX_ARRAY_STRIDE", GL_VERTEX_ARRAY_STRIDE, NUMBER, 1, EQ, {0}},
    {"NORMAL_ARRAY", GL_NORMAL_ARRAY, ENABLED, 1, EQ, {0}},
    {"NORMAL_ARRAY_TYPE", GL_NORMAL_ARRAY_TYPE, ENUMERATION, 1, EQ,
     {GL_FIXED}},
    {"NORMAL_ARRAY_STRIDE", GL_NORMAL_ARRAY_STRIDE, NUMBER, 1, EQ, {0}},
    {"COLOR_ARRAY", GL_COLOR_ARRAY, ENABLED, 1, EQ, {0}},
    {"COLOR_ARRAY_SIZE", GL_COLOR_ARRAY_SIZE, NUMBER, 1, EQ, {4}},
    {"COLOR_ARRAY_TYPE", GL_COLOR_ARRAY_TYPE, ENUMERATION, 1, EQ,
     {GL_FIXED}},
    {"COLOR_ARRAY_STRIDE", GL_COLOR_ARRAY_STRIDE, NUMBER, 1, EQ, {0}},
    {"TEXTURE_COORD_ARRAY", GL_TEXTURE_COORD_ARRAY, ENABLED, 1, EQ, {0}},
    {"TEXTURE_COORD_ARRAY_SIZE", GL_TEXTURE_COORD_ARRAY_SIZE, NUMBER, 1, EQ,
     {4}},
    {"TEXTURE_COORD_ARRAY_TYPE", GL_TEXTURE_COORD_ARRAY_TYPE, ENUMERATION, 1,
     EQ, {GL_FIXED}},
    {"TEXTURE_COORD_ARRAY_STRIDE", GL_TEXTURE_COORD_ARRAY_STRIDE, NUMBER, 1,
     EQ, {0}},
    {"POINT_SIZE_ARRAY_OES", GL_POINT_SIZE_ARRAY_OES, ENABLED, 1, EQ, {0}},
    {"POINT_SIZE_ARRAY_TYPE_OES", GL_POINT_SIZE_ARRAY_TYPE_OES, ENUMERATION,
     1, EQ, {GL_FIXED}},
    {"POINT_SIZE_ARRAY_STRIDE_OES", GL_POINT_SIZE_ARRAY_STRIDE_OES, NUMBER, 1,
     EQ, {0}},
    {"ARRAY_BUFFER_BINDING", GL_ARRAY_BUFFER_BINDING, NUMBER, 1, EQ, {0}},
    {"VERTEX_ARRAY_BUFFER_BINDING", GL_VERTEX_ARRAY_BUFFER_BINDING, NUMBER, 1,
     EQ, {0}},
    {"NORMAL_ARRAY_BUFFER_BINDING", GL_NORMAL_ARRAY_BUFFER_BINDING, NUMBER, 1,
     EQ, {0}},
    {"COLOR_ARRAY_BUFFER_BINDING", GL_COLOR_ARRAY_BUFFER_BINDING, NUMBER, 1,
     EQ, {0}},
    {"TEXTURE_COORD_ARRAY_BUFFER_BINDING",
     GL_TEXTURE_COORD_ARRAY_BUFFER_BINDING, NUMBER, 1, EQ, {0}},
    {"POINT_SIZE_ARRAY_BUFFER_BINDING_OES",
     GL_POINT_SIZE_ARRAY_BUFFER_BINDING_OES, NUMBER, 1, EQ, {0}},
    {"ELEMENT_ARRAY_BUFFER_BINDING", GL_ELEMENT_ARRAY_BUFFER_BINDING, NUMBER,
     1, EQ, {0}},
    /* Table 6.7, transformation state. */
    {"MODELVIEW_MATRIX", GL_MODELVIEW_MATRIX, FIXED, 16, EQ, IDENTITY},
    {"PROJECTION_MATRIX", GL_PROJECTION_MATRIX, FIXED, 16, EQ, IDENTITY},
    {"TEXTURE_MATRIX", GL_TEXTURE_MATRIX, FIXED, 16, EQ, IDENTITY},
    {"VIEWPORT", GL_VIEWPORT, NUMBER, 4, EQ,
     {0, 0, CONFORM_WIDTH, CONFORM_HEIGHT}},
    {"DEPTH_RANGE", GL_DEPTH_RANGE, UNIT, 2, EQ, {0, ONE}},
    {"MODELVIEW_STACK_DEPTH", GL_MODELVIEW_STACK_DEPTH, NUMBER, 1, EQ, {1}},
    {"PROJECTION_STACK_DEPTH", GL_PROJECTION_STACK_DEPTH, NUMBER, 1, EQ, {1}},
    {"TEXTURE_STACK_DEPTH", GL_TEXTURE_STACK_DEPTH, NUMBER, 1, EQ, {1}},
    {"MATRIX_MODE", GL_MATRIX_MODE, ENUMERATION, 1, EQ, {GL_MODELVIEW}},
    {"NORMALIZE", GL_NORMALIZE, ENABLED, 1, EQ, {0}},
    {"RESCALE_NORMAL", GL_RESCALE_NORMAL, ENABLED, 1, EQ, {0}},
    {"CLIP_PLANE0", GL_CLIP_PLANE0, ENABLED, 1, EQ, {0}},
    /* Table 6.8, colouring. */
    {"FOG_COLOR", GL_FOG_COLOR, UNIT, 4, EQ, {0, 0, 0, 0}},
    {"FOG_DENSITY", GL_FOG_DENSITY, FIXED, 1, EQ, {ONE}},
    {"FOG_START", GL_FOG_START, FIXED, 1, EQ, {0}},
    {"FOG_END", GL_FOG_END, FIXED, 1, EQ, {ONE}},
    {"FOG_MODE", GL_FOG_MODE, ENUMERATION, 1, EQ, {GL_EXP}},
    {"FOG", GL_FOG, ENABLED, 1, EQ, {0}},
    {"SHADE_MODEL", GL_SHADE_MODEL, ENUMERATION, 1, EQ, {GL_SMOOTH}},
    /* Table 6.9, lighting; the materials and lights are below. */
    {"LIGHTING", GL_LIGHTING, ENABLED, 1, EQ, {0}},
    {"COLOR_MATERIAL", GL_COLOR_MATERIAL, ENABLED, 1, EQ, {0}},
    {"LIGHT_MODEL_AMBIENT", GL_LIGHT_MODEL_AMBIENT, UNIT, 4, EQ,
     {13107, 13107, 13107, ONE}},
    {"LIGHT_MODEL_TWO_SIDE", GL_LIGHT_MODEL_TWO_SIDE, BOOLEAN, 1, EQ, {0}},
    {"LIGHT0", GL_LIGHT0, ENABLED, 1, EQ, {0}},
    {"LIGHT7", GL_LIGHT7, ENABLED, 1, EQ, {0}},
    /* Table 6.11, rasterisation. */
    {"POINT_SIZE", GL_POINT_SIZE, FIXED, 1, EQ, {ONE}},
    {"POINT_SMOOTH", GL_POINT_SMOOTH, ENABLED, 1, EQ, {0}},
    {"POINT_SIZE_MIN", GL_POINT_SIZE_MIN, FIXED, 1, EQ, {0}},
    {"POINT_FADE_THRESHOLD_SIZE", GL_POINT_FADE_THRESHOLD_SIZE, FIXED, 1, EQ,
     {ONE}},
    {"POINT_DISTANCE_ATTENUATION", GL_POINT_DISTANCE_ATTENUATION, FIXED, 3,
     EQ, {ONE, 0, 0}},
    {"POINT_SPRITE_OES", GL_POINT_SPRITE_OES, ENABLED, 1, EQ, {0}},
    {"LINE_WIDTH", GL_LINE_WIDTH, FIXED, 1, EQ, {ONE}},
    {"LINE_SMOOTH", GL_LINE_SMOOTH, ENABLED, 1, EQ, {0}},
    {"CULL_FACE", GL_CULL_FACE, ENABLED, 1, EQ, {0}},
    {"CULL_FACE_MODE", GL_CULL_FACE_MODE, ENUMERATION, 1, EQ, {GL_BACK}},
    {"FRONT_FACE", GL_FRONT_FACE, ENUMERATION, 1, EQ, {GL_CCW}},
    {"POLYGON_OFFSET_FACTOR", GL_POLYGON_OFFSET_FACTOR, FIXED, 1, EQ, {0}},
    {"POLYGON_OFFSET_UNITS", GL_POLYGON_OFFSET_UNITS, FIXED, 1, EQ, {0}},
    {"POLYGON_OFFSET_FILL", GL_POLYGON_OFFSET_FILL, ENABLED, 1, EQ, {0}},
    /* Table 6.12, multisampling. */
    {"MULTISAMPLE", GL_MULTISAMPLE, ENABLED, 1, EQ, {1}},
    {"SAMPLE_ALPHA_TO_COVERAGE", GL_SAMPLE_ALPHA_TO_COVERAGE, ENABLED, 1, EQ,
     {0}},
    {"SAMPLE_ALPHA_TO_ONE", GL_SAMPLE_ALPHA_TO_ONE, ENABLED, 1, EQ, {0}},
    {"SAMPLE_COVERAGE", GL_SAMPLE_COVERAGE, ENABLED, 1, EQ, {0}},
    {"SAMPLE_COVERAGE_VALUE", GL_SAMPLE_COVERAGE_VALUE, FIXED, 1, EQ, {ONE}},
    {"SAMPLE_COVERAGE_INVERT", GL_SAMPLE_COVERAGE_INVERT, BOOLEAN, 1, EQ,
     {0}},
    /* Table 6.13, textures per unit; the objects' are below. */
    {"TEXTURE_2D", GL_TEXTURE_2D, ENABLED, 1, EQ, {0}},
    {"TEXTURE_BINDING_2D", GL_TEXTURE_BINDING_2D, NUMBER, 1, EQ, {0}},
    /* Table 6.15, the active unit; the environment is below. */
    {"ACTIVE_TEXTURE", GL_ACTIVE_TEXTURE, ENUMERATION, 1, EQ, {GL_TEXTURE0}},
    /* Table 6.16, pixel operations. */
    {"SCISSOR_TEST", GL_SCISSOR_TEST, ENABLED, 1, EQ, {0}},
    {"SCISSOR_BOX", GL_SCISSOR_BOX, NUMBER, 4, EQ,
     {0, 0, CONFORM_WIDTH, CONFORM_HEIGHT}},
    {"ALPHA_TEST", GL_ALPHA_TEST, ENABLED, 1, EQ, {0}},
    {"ALPHA_TEST_FUNC", GL_ALPHA_TEST_FUNC, ENUMERATION, 1, EQ, {GL_ALWAYS}},
    {"ALPHA_TEST_REF", GL_ALPHA_TEST_REF, UNIT, 1, EQ, {0}},
    {"STENCIL_TEST", GL_STENCIL_TEST, ENABLED, 1, EQ, {0}},
    {"STENCIL_FUNC", GL_STENCIL_FUNC, ENUMERATION, 1, EQ, {GL_ALWAYS}},
    {"STENCIL_VALUE_MASK", GL_STENCIL_VALUE_MASK, NUMBER, 1, ONES, {0}},
    {"STENCIL_REF", GL_STENCIL_REF, NUMBER, 1, EQ, {0}},
    {"STENCIL_FAIL", GL_STENCIL_FAIL, ENUMERATION, 1, EQ, {GL_KEEP}},
    {"STENCIL_PASS_DEPTH_FAIL", GL_STENCIL_PASS_DEPTH_FAIL, ENUMERATION, 1,
     EQ, {GL_KEEP}},
    {"STENCIL_PASS_DEPTH_PASS", GL_STENCIL_PASS_DEPTH_PASS, ENUMERATION, 1,
     EQ, {GL_KEEP}},
    {"DEPTH_TEST", GL_DEPTH_TEST, ENABLED, 1, EQ, {0}},
    {"DEPTH_FUNC", GL_DEPTH_FUNC, ENUMERATION, 1, EQ, {GL_LESS}},
    {"BLEND", GL_BLEND, ENABLED, 1, EQ, {0}},
    {"BLEND_SRC", GL_BLEND_SRC, ENUMERATION, 1, EQ, {GL_ONE}},
    {"BLEND_DST", GL_BLEND_DST, ENUMERATION, 1, EQ, {GL_ZERO}},
    {"DITHER", GL_DITHER, ENABLED, 1, EQ, {1}},
    {"COLOR_LOGIC_OP", GL_COLOR_LOGIC_OP, ENABLED, 1, EQ, {0}},
    {"LOGIC_OP_MODE", GL_LOGIC_OP_MODE, ENUMERATION, 1, EQ, {GL_COPY}},
    /* Table 6.17, framebuffer control. */
    {"COLOR_WRITEMASK", GL_COLOR_WRITEMASK, BOOLEAN, 4, EQ, {1, 1, 1, 1}},
    {"DEPTH_WRITEMASK", GL_DEPTH_WRITEMASK, BOOLEAN, 1, EQ, {1}},
    {"STENCIL_WRITEMASK", GL_STENCIL_WRITEMASK, NUMBER, 1, ONES, {0}},
    {"COLOR_CLEAR_VALUE", GL_COLOR_CLEAR_VALUE, UNIT, 4, EQ, {0, 0, 0, 0}},
    {"DEPTH_CLEAR_VALUE", GL_DEPTH_CLEAR_VALUE, UNIT, 1, EQ, {ONE}},
    {"STENCIL_CLEAR_VALUE", GL_STENCIL_CLEAR_VALUE, NUMBER, 1, EQ, {0}},
    /* Table 6.18, pixels. */
    {"UNPACK_ALIGNMENT", GL_UNPACK_ALIGNMENT, NUMBER, 1, EQ, {4}},
    {"PACK_ALIGNMENT", GL_PACK_ALIGNMENT, NUMBER, 1, EQ, {4}},
    /* Table 6.19, hints. */
    {"PERSPECTIVE_CORRECTION_HINT", GL_PERSPECTIVE_CORRECTION_HINT,
     ENUMERATION, 1, EQ, {GL_DONT_CARE}},
    {"POINT_SMOOTH_HINT", GL_POINT_SMOOTH_HINT, ENUMERATION, 1, EQ,
     {GL_DONT_CARE}},
    {"LINE_SMOOTH_HINT", GL_LINE_SMOOTH_HINT, ENUMERATION, 1, EQ,
     {GL_DONT_CARE}},
    {"FOG_HINT", GL_FOG_HINT, ENUMERATION, 1, EQ, {GL_DONT_CARE}},
    {"GENERATE_MIPMAP_HINT", GL_GENERATE_MIPMAP_HINT, ENUMERATION, 1, EQ,
     {GL_DONT_CARE}},
    /* Tables 6.20 to 6.22, implementation-dependent values, each at least
     * the table's minimum. */
    {"MAX_LIGHTS", GL_MAX_LIGHTS, NUMBER, 1, AT_LEAST, {8}},
    {"MAX_CLIP_PLANES", GL_MAX_CLIP_PLANES, NUMBER, 1, AT_LEAST, {1}},
    {"MAX_MODELVIEW_STACK_DEPTH", GL_MAX_MODELVIEW_STACK_DEPTH, NUMBER, 1,
     AT_LEAST, {16}},
    {"MAX_PROJECTION_STACK_DEPTH", GL_MAX_PROJECTION_STACK_DEPTH, NUMBER, 1,
     AT_LEAST, {2}},
    {"MAX_TEXTURE_STACK_DEPTH", GL_MAX_TEXTURE_STACK_DEPTH, NUMBER, 1,
     AT_LEAST, {2}},
    {"SUBPIXEL_BITS", GL_SUBPIXEL_BITS, NUMBER, 1, AT_LEAST, {4}},
    {"MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, NUMBER, 1, AT_LEAST, {64}},
    {"MAX_VIEWPORT_DIMS", GL_MAX_VIEWPORT_DIMS, NUMBER, 2, AT_LEAST,
     {CONFORM_WIDTH, CONFORM_HEIGHT}},
    {"ALIASED_POINT_SIZE_RANGE", GL_ALIASED_POINT_SIZE_RANGE, FIXED, 2,
     RANGE, {ONE, ONE}},
    {"SMOOTH_POINT_SIZE_RANGE", GL_SMOOTH_POINT_SIZE_RANGE, FIXED, 2, RANGE,
     {ONE, ONE}},
    {"ALIASED_LINE_WIDTH_RANGE", GL_ALIASED_LINE_WIDTH_RANGE, FIXED, 2, RANGE,
     {ONE, ONE}},
    {"SMOOTH_LINE_WIDTH_RANGE", GL_SMOOTH_LINE_WIDTH_RANGE, FIXED, 2, RANGE,
     {ONE, ONE}},
    {"MAX_TEXTURE_UNITS", GL_MAX_TEXTURE_UNITS, NUMBER, 1, AT_LEAST, {2}},
    {"SAMPLE_BUFFERS", GL_SAMPLE_BUFFERS, NUMBER, 1, AT_LEAST, {0}},
    {"SAMPLES", GL_SAMPLES, NUMBER, 1, AT_LEAST, {0}},
    {"NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS,
     NUMBER, 1, AT_LEAST, {10}},
    /* Table 6.23, the pixel depths, as the configuration asked for. */
    {"RED_BITS", GL_RED_BITS, NUMBER, 1, AT_LEAST, {8}},
    {"GREEN_BITS", GL_GREEN_BITS, NUMBER, 1, AT_LEAST, {8}},
    {"BLUE_BITS", GL_BLUE_BITS, NUMBER, 1, AT_LEAST, {8}},
    {"DEPTH_BITS", GL_DEPTH_BITS, NUMBER, 1, AT_LEAST, {16}},
    {"STENCIL_BITS", GL_STENCIL_BITS, NUMBER, 1, AT_LEAST, {8}},
};

/* A unit value in 16.16 as GetIntegerv gives it, [-1, 1] onto the
 * integers' range. */
static long long unit_int(long long v) {
  return (v * 2147483647LL) / ONE;
}

/* Whether `got` holds `want` as `hold` says, element `k` of a row of
 * `n`; fixed point within one unit in the last place. */
static int holds(enum hold hold, enum kind kind, int k, long long got,
                 long long want) {
  long long slack = (kind == FIXED || kind == UNIT) ? 1 : 0;
  switch (hold) {
  case EQ:
    return got >= want - slack && got <= want + slack;
  case AT_LEAST:
    return got >= want;
  case RANGE:
    return k == 0 ? got <= want : got >= want;
  case ONES:
    return (got & 0xff) == 0xff;
  }
  return 0;
}

/* The row read through `kind`'s own query into `out`, as long longs. */
static void read_as(const struct row *r, enum kind as, long long *out) {
  GLint i[16];
  GLfixed x[16];
  GLboolean b[16];
  memset(i, 0, sizeof i);
  memset(x, 0, sizeof x);
  memset(b, 0, sizeof b);
  switch (as) {
  case ENABLED:
    out[0] = glIsEnabled(r->pname);
    return;
  case BOOLEAN:
    glGetBooleanv(r->pname, b);
    for (int k = 0; k < r->n; k++)
      out[k] = b[k];
    return;
  case NUMBER:
  case ENUMERATION:
    glGetIntegerv(r->pname, i);
    for (int k = 0; k < r->n; k++)
      out[k] = i[k];
    return;
  case FIXED:
  case UNIT:
    glGetFixedv(r->pname, x);
    for (int k = 0; k < r->n; k++)
      out[k] = x[k];
    return;
  }
}

/* What the row's value `w` is, read through `as` (section 6.1.2). */
static long long converted(enum kind kind, enum kind as, long long w) {
  if (as == BOOLEAN || as == ENABLED)
    return w != 0;
  switch (kind) {
  case ENABLED:
  case BOOLEAN:
    return as == FIXED ? w * ONE : w;
  case NUMBER:
    return as == FIXED ? w * ONE : w;
  case ENUMERATION:
    return w;
  case FIXED:
    return as == NUMBER ? (w >= 0 ? (w + ONE / 2) / ONE : -((-w + ONE / 2) / ONE))
                        : w;
  case UNIT:
    return as == NUMBER ? unit_int(w) : w;
  }
  return w;
}

/* The row through its own query, and, held equal, through the others. */
static void check(const struct row *r) {
  long long got[16];
  char why[256];
  int ok = 1;
  why[0] = 0;
  glGetError();
  read_as(r, r->kind, got);
  for (int k = 0; k < r->n && ok; k++) {
    if (!holds(r->hold, r->kind, k, got[k], r->want[k])) {
      ok = 0;
      snprintf(why, sizeof why, "element %d is %lld, not %lld", k, got[k],
               r->want[k]);
    }
  }
  GLenum e = glGetError();
  if (ok && e != GL_NO_ERROR) {
    ok = 0;
    snprintf(why, sizeof why, "the query set error %04x", e);
  }
  /* The other queries, for a value held equal. */
  enum kind others[3];
  int m = 0;
  if (r->hold == EQ) {
    if (r->kind != BOOLEAN)
      others[m++] = BOOLEAN;
    if (r->kind != NUMBER && r->kind != ENUMERATION)
      others[m++] = NUMBER;
    if (r->kind != FIXED && r->kind != UNIT)
      others[m++] = FIXED;
  }
  static const char *query[] = {"IsEnabled",   "GetBooleanv",
                                "GetIntegerv", "GetIntegerv",
                                "GetFixedv",   "GetFixedv"};
  for (int a = 0; a < m && ok; a++) {
    long long alt[16];
    struct row as = *r;
    read_as(&as, others[a], alt);
    GLenum ae = glGetError();
    if (ae != GL_NO_ERROR) {
      ok = 0;
      snprintf(why, sizeof why, "%s set error %04x", query[others[a]], ae);
    }
    for (int k = 0; k < r->n && ok; k++) {
      long long want = converted(r->kind, others[a], r->want[k]);
      long long slack = (r->kind == UNIT && others[a] == NUMBER) ? 65536 : 0;
      if (r->kind == FIXED && others[a] == FIXED)
        slack = 1;
      if (alt[k] < want - slack || alt[k] > want + slack) {
        ok = 0;
        snprintf(why, sizeof why, "%s gives element %d as %lld, not %lld",
                 query[others[a]], k, alt[k], want);
      }
    }
  }
  if (ok)
    conform_pass("state", r->name);
  else
    conform_fail("state", r->name, "%s", why);
}

/* A query of a parameter of an object, light, material, texture or
 * environment, of `n` values in 16.16, against `want`. */
static void params(const char *name, int n, const GLfixed *got,
                   const long long *want) {
  GLenum e = glGetError();
  for (int k = 0; k < n; k++) {
    if (got[k] < want[k] - 1 || got[k] > want[k] + 1) {
      conform_fail("state", name, "element %d is %d, not %lld", k, got[k],
                   want[k]);
      return;
    }
  }
  if (e != GL_NO_ERROR) {
    conform_fail("state", name, "the query set error %04x", e);
    return;
  }
  conform_pass("state", name);
}

/* The same for an integer parameter. */
static void iparams(const char *name, int n, const GLint *got,
                    const long long *want) {
  GLenum e = glGetError();
  for (int k = 0; k < n; k++) {
    if (got[k] != want[k]) {
      conform_fail("state", name, "element %d is %d, not %lld", k, got[k],
                   want[k]);
      return;
    }
  }
  if (e != GL_NO_ERROR) {
    conform_fail("state", name, "the query set error %04x", e);
    return;
  }
  conform_pass("state", name);
}

/* Tables 6.9 and 6.10: the materials, both faces, and lights 0 and 7,
 * as Table 2.8 gives the first light's diffuse and specular and the
 * others'. */
static void lighting(void) {
  static const struct {
    const char *name;
    GLenum pname;
    int n;
    long long want[4];
  } material[] = {
      {"AMBIENT", GL_AMBIENT, 4, {13107, 13107, 13107, ONE}},
      {"DIFFUSE", GL_DIFFUSE, 4, {52429, 52429, 52429, ONE}},
      {"SPECULAR", GL_SPECULAR, 4, {0, 0, 0, ONE}},
      {"EMISSION", GL_EMISSION, 4, {0, 0, 0, ONE}},
      {"SHININESS", GL_SHININESS, 1, {0}},
  };
  static const GLenum faces[2] = {GL_FRONT, GL_BACK};
  static const char *face_name[2] = {"FRONT", "BACK"};
  char name[96];
  for (unsigned m = 0; m < sizeof material / sizeof material[0]; m++) {
    for (int f = 0; f < 2; f++) {
      GLfixed got[4] = {0x7eadbeef, 0x7eadbeef, 0x7eadbeef, 0x7eadbeef};
      glGetError();
      glGetMaterialxv(faces[f], material[m].pname, got);
      snprintf(name, sizeof name, "MATERIAL_%s_%s", face_name[f],
               material[m].name);
      params(name, material[m].n, got, material[m].want);
    }
  }
  static const struct {
    const char *name;
    GLenum pname;
    int n;
    long long first[4], rest[4];
  } light[] = {
      {"AMBIENT", GL_AMBIENT, 4, {0, 0, 0, ONE}, {0, 0, 0, ONE}},
      {"DIFFUSE", GL_DIFFUSE, 4, {ONE, ONE, ONE, ONE}, {0, 0, 0, ONE}},
      {"SPECULAR", GL_SPECULAR, 4, {ONE, ONE, ONE, ONE}, {0, 0, 0, ONE}},
      {"POSITION", GL_POSITION, 4, {0, 0, ONE, 0}, {0, 0, ONE, 0}},
      {"CONSTANT_ATTENUATION", GL_CONSTANT_ATTENUATION, 1, {ONE}, {ONE}},
      {"LINEAR_ATTENUATION", GL_LINEAR_ATTENUATION, 1, {0}, {0}},
      {"QUADRATIC_ATTENUATION", GL_QUADRATIC_ATTENUATION, 1, {0}, {0}},
      {"SPOT_DIRECTION", GL_SPOT_DIRECTION, 3, {0, 0, -ONE}, {0, 0, -ONE}},
      {"SPOT_EXPONENT", GL_SPOT_EXPONENT, 1, {0}, {0}},
      {"SPOT_CUTOFF", GL_SPOT_CUTOFF, 1, {180 * ONE}, {180 * ONE}},
  };
  for (unsigned l = 0; l < sizeof light / sizeof light[0]; l++) {
    for (int i = 0; i < 8; i += 7) {
      GLfixed got[4] = {0x7eadbeef, 0x7eadbeef, 0x7eadbeef, 0x7eadbeef};
      glGetError();
      glGetLightxv(GL_LIGHT0 + i, light[l].pname, got);
      snprintf(name, sizeof name, "LIGHT%d_%s", i, light[l].name);
      params(name, light[l].n, got, i == 0 ? light[l].first : light[l].rest);
    }
  }
}

/* Table 6.14, the default texture object's parameters, and Table 6.15,
 * the environment's, through their own queries. */
static void texturing(void) {
  static const struct {
    const char *name;
    GLenum pname;
    long long want;
  } tex[] = {
      {"TEXTURE_MIN_FILTER", GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_LINEAR},
      {"TEXTURE_MAG_FILTER", GL_TEXTURE_MAG_FILTER, GL_LINEAR},
      {"TEXTURE_WRAP_S", GL_TEXTURE_WRAP_S, GL_REPEAT},
      {"TEXTURE_WRAP_T", GL_TEXTURE_WRAP_T, GL_REPEAT},
      {"GENERATE_MIPMAP", GL_GENERATE_MIPMAP, GL_FALSE},
  };
  char name[96];
  for (unsigned t = 0; t < sizeof tex / sizeof tex[0]; t++) {
    GLint got = 0x7eadbeef;
    glGetError();
    glGetTexParameteriv(GL_TEXTURE_2D, tex[t].pname, &got);
    iparams(tex[t].name, 1, &got, &tex[t].want);
  }
  static const struct {
    const char *name;
    GLenum target, pname;
    long long want;
  } env[] = {
      {"TEXTURE_ENV_MODE", GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, GL_MODULATE},
      {"COORD_REPLACE_OES", GL_POINT_SPRITE_OES, GL_COORD_REPLACE_OES,
       GL_FALSE},
      {"COMBINE_RGB", GL_TEXTURE_ENV, GL_COMBINE_RGB, GL_MODULATE},
      {"COMBINE_ALPHA", GL_TEXTURE_ENV, GL_COMBINE_ALPHA, GL_MODULATE},
      {"SRC0_RGB", GL_TEXTURE_ENV, GL_SRC0_RGB, GL_TEXTURE},
      {"SRC1_RGB", GL_TEXTURE_ENV, GL_SRC1_RGB, GL_PREVIOUS},
      {"SRC2_RGB", GL_TEXTURE_ENV, GL_SRC2_RGB, GL_CONSTANT},
      {"SRC0_ALPHA", GL_TEXTURE_ENV, GL_SRC0_ALPHA, GL_TEXTURE},
      {"SRC1_ALPHA", GL_TEXTURE_ENV, GL_SRC1_ALPHA, GL_PREVIOUS},
      {"SRC2_ALPHA", GL_TEXTURE_ENV, GL_SRC2_ALPHA, GL_CONSTANT},
      {"OPERAND0_RGB", GL_TEXTURE_ENV, GL_OPERAND0_RGB, GL_SRC_COLOR},
      {"OPERAND1_RGB", GL_TEXTURE_ENV, GL_OPERAND1_RGB, GL_SRC_COLOR},
      {"OPERAND2_RGB", GL_TEXTURE_ENV, GL_OPERAND2_RGB, GL_SRC_ALPHA},
      {"OPERAND0_ALPHA", GL_TEXTURE_ENV, GL_OPERAND0_ALPHA, GL_SRC_ALPHA},
      {"OPERAND1_ALPHA", GL_TEXTURE_ENV, GL_OPERAND1_ALPHA, GL_SRC_ALPHA},
      {"OPERAND2_ALPHA", GL_TEXTURE_ENV, GL_OPERAND2_ALPHA, GL_SRC_ALPHA},
  };
  for (unsigned t = 0; t < sizeof env / sizeof env[0]; t++) {
    GLint got = 0x7eadbeef;
    glGetError();
    glGetTexEnviv(env[t].target, env[t].pname, &got);
    iparams(env[t].name, 1, &got, &env[t].want);
  }
  static const long long zero4[4] = {0, 0, 0, 0}, one1[1] = {ONE};
  GLfixed got[4] = {0x7eadbeef, 0x7eadbeef, 0x7eadbeef, 0x7eadbeef};
  glGetError();
  glGetTexEnvxv(GL_TEXTURE_ENV, GL_TEXTURE_ENV_COLOR, got);
  params("TEXTURE_ENV_COLOR", 4, got, zero4);
  glGetError();
  glGetTexEnvxv(GL_TEXTURE_ENV, GL_RGB_SCALE, got);
  params("RGB_SCALE", 1, got, one1);
  glGetError();
  glGetTexEnvxv(GL_TEXTURE_ENV, GL_ALPHA_SCALE, got);
  params("ALPHA_SCALE", 1, got, one1);
  (void)name;
}

/* The rest: the clip plane, the arrays' pointers, a buffer object's
 * state, the compressed formats, the largest point size, and the first
 * error, which is none. */
static void others(void) {
  static const long long zero4[4] = {0, 0, 0, 0};
  GLfixed eq[4] = {0x7eadbeef, 0x7eadbeef, 0x7eadbeef, 0x7eadbeef};
  glGetError();
  glGetClipPlanex(GL_CLIP_PLANE0, eq);
  params("CLIP_PLANE0_EQUATION", 4, eq, zero4);
  static const struct {
    const char *name;
    GLenum pname;
  } pointers[] = {
      {"VERTEX_ARRAY_POINTER", GL_VERTEX_ARRAY_POINTER},
      {"NORMAL_ARRAY_POINTER", GL_NORMAL_ARRAY_POINTER},
      {"COLOR_ARRAY_POINTER", GL_COLOR_ARRAY_POINTER},
      {"TEXTURE_COORD_ARRAY_POINTER", GL_TEXTURE_COORD_ARRAY_POINTER},
      {"POINT_SIZE_ARRAY_POINTER_OES", GL_POINT_SIZE_ARRAY_POINTER_OES},
  };
  for (unsigned p = 0; p < sizeof pointers / sizeof pointers[0]; p++) {
    void *at = (void *)1;
    glGetError();
    glGetPointerv(pointers[p].pname, &at);
    GLenum e = glGetError();
    if (at == 0 && e == GL_NO_ERROR)
      conform_pass("state", pointers[p].name);
    else
      conform_fail("state", pointers[p].name, "%p, error %04x", at, e);
  }
  GLuint name = 0;
  glGenBuffers(1, &name);
  glBindBuffer(GL_ARRAY_BUFFER, name);
  GLint size = -1, usage = -1;
  glGetError();
  glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &size);
  const long long zero1[1] = {0}, static1[1] = {GL_STATIC_DRAW};
  iparams("BUFFER_SIZE", 1, &size, zero1);
  glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_USAGE, &usage);
  iparams("BUFFER_USAGE", 1, &usage, static1);
  glBindBuffer(GL_ARRAY_BUFFER, 0);
  glDeleteBuffers(1, &name);
  /* The ten paletted formats, among those the implementation names. */
  GLint count = 0, formats[64];
  glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &count);
  if (count > 64)
    count = 64;
  glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, formats);
  int found = 0;
  for (GLenum f = GL_PALETTE4_RGB8_OES; f <= GL_PALETTE8_RGB5_A1_OES; f++)
    for (int k = 0; k < count; k++)
      found += formats[k] == (GLint)f;
  if (found == 10)
    conform_pass("state", "COMPRESSED_TEXTURE_FORMATS");
  else
    conform_fail("state", "COMPRESSED_TEXTURE_FORMATS",
                 "%d of the ten paletted formats in %d", found, count);
  /* POINT_SIZE_MAX starts at the larger of the two ranges' largest. */
  GLfixed aliased[2], smooth[2], most = -1;
  glGetFixedv(GL_ALIASED_POINT_SIZE_RANGE, aliased);
  glGetFixedv(GL_SMOOTH_POINT_SIZE_RANGE, smooth);
  glGetError();
  glGetFixedv(GL_POINT_SIZE_MAX, &most);
  GLfixed want = aliased[1] > smooth[1] ? aliased[1] : smooth[1];
  GLenum e = glGetError();
  if (most == want && e == GL_NO_ERROR)
    conform_pass("state", "POINT_SIZE_MAX");
  else
    conform_fail("state", "POINT_SIZE_MAX", "%d, not %d, error %04x", most,
                 want, e);
}

/* Section 6.1.2 and Table C.2: each matrix as the bits of the
 * single-precision floats its elements are, through GetIntegerv. */
static void matrix_bits(void) {
  static const struct {
    const char *name;
    GLenum mode, pname;
  } m[3] = {
      {"MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES", GL_MODELVIEW,
       GL_MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES},
      {"PROJECTION_MATRIX_FLOAT_AS_INT_BITS_OES", GL_PROJECTION,
       GL_PROJECTION_MATRIX_FLOAT_AS_INT_BITS_OES},
      {"TEXTURE_MATRIX_FLOAT_AS_INT_BITS_OES", GL_TEXTURE,
       GL_TEXTURE_MATRIX_FLOAT_AS_INT_BITS_OES},
  };
  GLfixed load[16];
  for (int k = 0; k < 16; k++)
    load[k] = (GLfixed)((k + 1) * ONE / 4);
  for (int r = 0; r < 3; r++) {
    glMatrixMode(m[r].mode);
    glLoadMatrixx(load);
    GLint got[16];
    memset(got, 0, sizeof got);
    glGetError();
    glGetIntegerv(m[r].pname, got);
    GLenum e = glGetError();
    int k = 0;
    for (; k < 16; k++) {
      float f = (float)(k + 1) / 4;
      GLint bits;
      memcpy(&bits, &f, sizeof bits);
      if (got[k] != bits)
        break;
    }
    glLoadIdentity();
    if (e != GL_NO_ERROR)
      conform_fail("state", m[r].name, "error %04x", e);
    else if (k < 16)
      conform_fail("state", m[r].name, "element %d is %08x", k, got[k]);
    else
      conform_pass("state", m[r].name);
  }
  glMatrixMode(GL_MODELVIEW);
}

/* Appendix C.3: the required profile extensions are in the EXTENSIONS
 * string, each a word of it. */
static void extensions(void) {
  static const char *required[4] = {
      "GL_OES_read_format", "GL_OES_compressed_paletted_texture",
      "GL_OES_point_size_array", "GL_OES_point_sprite"};
  const char *s = (const char *)glGetString(GL_EXTENSIONS);
  for (int k = 0; k < 4; k++) {
    size_t n = strlen(required[k]);
    int found = 0;
    for (const char *p = s; p && *p && !found;) {
      const char *end = strchr(p, ' ');
      size_t len = end ? (size_t)(end - p) : strlen(p);
      found = len == n && strncmp(p, required[k], n) == 0;
      p = end ? end + 1 : p + len;
    }
    char name[64];
    snprintf(name, sizeof name, "EXTENSION_%s", required[k] + 3);
    if (found)
      conform_pass("state", name);
    else
      conform_fail("state", name, "not in \"%s\"", s ? s : "(null)");
  }
}

void conform_state(void) {
  /* Table 6.24: no error before any call has made one. */
  GLenum first = glGetError();
  if (first == GL_NO_ERROR)
    conform_pass("state", "GET_ERROR");
  else
    conform_fail("state", "GET_ERROR", "%04x", first);
  for (unsigned r = 0; r < sizeof rows / sizeof rows[0]; r++)
    check(&rows[r]);
  lighting();
  texturing();
  others();
  matrix_bits();
  extensions();
}
