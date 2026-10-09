/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The errors (#999): for each command, the error GL ES 1.1's specification,
 * version 1.1.12, says a bad argument gives, as its section 2.5 and each
 * command's section state them. Section 2.5's rules:
 *  - an enumerated argument GL does not allow is INVALID_ENUM;
 *  - a numeric argument out of its range is INVALID_VALUE;
 *  - a command illegal in the state it meets is INVALID_OPERATION;
 *  - a push onto a full stack is STACK_OVERFLOW, a pop of the last entry
 *    STACK_UNDERFLOW;
 *  - the command then does nothing but record the error, and a second
 *    GetError after it is NO_ERROR.
 * Some cases also check that the state the command would have set is as
 * it was.
 */
#include <stddef.h>
#include <stdio.h>

#include "conform.h"

#define ONE CONFORM_ONE

/* A case: its name, the call, and the error it must give. */
struct error_case {
  const char *name;
  void (*call)(void);
  GLenum want;
};

#define CALL(name, body)                                                       \
  static void call_##name(void) { body; }

static GLint ints[16];
static GLfixed fixeds[16];
static GLboolean booleans[16];
static GLubyte pixels[64];
static GLuint names[4];
static const GLshort quad[8] = {0, 0, 1, 0, 1, 1, 0, 1};

/* Section 2.5 and 6.1: switches and queries. */
CALL(enable_bad, glEnable(0x1234))
CALL(disable_bad, glDisable(0x1234))
CALL(is_enabled_bad, (void)glIsEnabled(0x1234))
CALL(get_integer_bad, glGetIntegerv(0x1234, ints))
CALL(get_fixed_bad, glGetFixedv(0x1234, fixeds))
CALL(get_boolean_bad, glGetBooleanv(0x1234, booleans))
CALL(get_string_bad, (void)glGetString(0x1234))
CALL(enable_client_state_bad, glEnableClientState(0x1234))
/* Section 2.8: vertex arrays, Table 2.4's sizes and types. */
CALL(vertex_pointer_size, glVertexPointer(1, GL_SHORT, 0, quad))
CALL(vertex_pointer_type, glVertexPointer(2, GL_UNSIGNED_BYTE, 0, quad))
CALL(vertex_pointer_stride, glVertexPointer(2, GL_SHORT, -1, quad))
CALL(color_pointer_size, glColorPointer(3, GL_UNSIGNED_BYTE, 0, quad))
CALL(color_pointer_type, glColorPointer(4, GL_BYTE, 0, quad))
CALL(normal_pointer_type, glNormalPointer(GL_UNSIGNED_BYTE, 0, quad))
CALL(tex_coord_pointer_size, glTexCoordPointer(1, GL_SHORT, 0, quad))
CALL(client_active_texture_bad, glClientActiveTexture(GL_TEXTURE0 + 32))
CALL(draw_arrays_mode, glDrawArrays(0x1234, 0, 3))
CALL(draw_arrays_count, glDrawArrays(GL_TRIANGLES, 0, -1))
CALL(draw_elements_mode, glDrawElements(0x1234, 3, GL_UNSIGNED_BYTE, quad))
CALL(draw_elements_count,
     glDrawElements(GL_TRIANGLES, -1, GL_UNSIGNED_BYTE, quad))
/* GL's UNSIGNED_INT, which ES 1.1 does not take for indices. */
CALL(draw_elements_type, glDrawElements(GL_TRIANGLES, 3, 0x1405, quad))
/* Section 2.9: buffer objects. */
CALL(bind_buffer_target, glBindBuffer(0x1234, 1))
CALL(gen_buffers_count, glGenBuffers(-1, names))
CALL(delete_buffers_count, glDeleteBuffers(-1, names))
/* Each with a buffer of four bytes bound, given back after. */
static void with_buffer(void (*call)(void)) {
  GLuint b = 0;
  glGenBuffers(1, &b);
  glBindBuffer(GL_ARRAY_BUFFER, b);
  glBufferData(GL_ARRAY_BUFFER, 4, 0, GL_STATIC_DRAW);
  while (glGetError() != GL_NO_ERROR)
    ;
  call();
  /* The error stays for the case to read; the clean-up makes none. */
  glBindBuffer(GL_ARRAY_BUFFER, 0);
  glDeleteBuffers(1, &b);
}
static void data_size(void) {
  glBufferData(GL_ARRAY_BUFFER, -1, 0, GL_STATIC_DRAW);
}
static void data_usage(void) { glBufferData(GL_ARRAY_BUFFER, 4, 0, 0x1234); }
static void sub_data_range(void) {
  glBufferSubData(GL_ARRAY_BUFFER, 2, 4, pixels);
}
CALL(buffer_data_size, with_buffer(data_size))
CALL(buffer_data_usage, with_buffer(data_usage))
CALL(buffer_sub_data_range, with_buffer(sub_data_range))
/* Section 2.10: the viewport, matrices and their stacks. */
CALL(viewport_negative, glViewport(0, 0, -1, 4))
CALL(matrix_mode_bad, glMatrixMode(0x1234))
CALL(frustum_near, glFrustumx(-ONE, ONE, -ONE, ONE, 0, ONE))
CALL(frustum_sides, glFrustumx(ONE, ONE, -ONE, ONE, ONE, 2 * ONE))
CALL(frustum_depth, glFrustumx(-ONE, ONE, -ONE, ONE, ONE, ONE))
CALL(ortho_sides, glOrthox(-ONE, ONE, ONE, ONE, 0, ONE))
CALL(ortho_depth, glOrthox(-ONE, ONE, -ONE, ONE, ONE, ONE))
CALL(pop_underflow, glPopMatrix())
/* Section 2.11: the clip plane. */
static const GLfixed plane[4] = {0, 0, ONE, 0};
CALL(clip_plane_bad, glClipPlanex(GL_CLIP_PLANE0 + 1, plane))
/* Section 2.12: lighting, Table 2.8's ranges. */
CALL(light_bad, glLightx(GL_LIGHT0 + 8, GL_SPOT_EXPONENT, 0))
CALL(light_pname, glLightx(GL_LIGHT0, 0x1234, 0))
CALL(light_vector_pname, glLightx(GL_LIGHT0, GL_POSITION, 0))
CALL(light_spot_exponent, glLightx(GL_LIGHT0, GL_SPOT_EXPONENT, 129 * ONE))
CALL(light_spot_cutoff, glLightx(GL_LIGHT0, GL_SPOT_CUTOFF, 91 * ONE))
CALL(light_attenuation, glLightx(GL_LIGHT0, GL_LINEAR_ATTENUATION, -ONE))
CALL(material_face, glMaterialx(GL_FRONT, GL_SHININESS, 0))
CALL(material_pname, glMaterialx(GL_FRONT_AND_BACK, 0x1234, 0))
CALL(material_vector_pname, glMaterialx(GL_FRONT_AND_BACK, GL_AMBIENT, 0))
CALL(material_shininess,
     glMaterialx(GL_FRONT_AND_BACK, GL_SHININESS, 129 * ONE))
CALL(light_model_pname, glLightModelx(0x1234, 0))
CALL(shade_model_bad, glShadeModel(0x1234))
/* Chapter 3: points, lines, polygons. */
CALL(point_size_zero, glPointSizex(0))
CALL(point_size_negative, glPointSizex(-ONE))
CALL(line_width_zero, glLineWidthx(0))
CALL(cull_face_bad, glCullFace(0x1234))
CALL(front_face_bad, glFrontFace(0x1234))
/* Section 3.6.1 and 4.3.1: pixel storage and reading back. */
CALL(pixel_store_pname, glPixelStorei(0x1234, 4))
CALL(pixel_store_value, glPixelStorei(GL_UNPACK_ALIGNMENT, 3))
CALL(read_pixels_format, glReadPixels(0, 0, 1, 1, GL_RGB, GL_UNSIGNED_BYTE,
                                      pixels))
CALL(read_pixels_type, glReadPixels(0, 0, 1, 1, GL_RGBA, GL_FIXED, pixels))
CALL(read_pixels_size, glReadPixels(0, 0, -1, 1, GL_RGBA, GL_UNSIGNED_BYTE,
                                    pixels))
/* Section 3.7: textures. */
CALL(bind_texture_target, glBindTexture(0x1234, 0))
CALL(gen_textures_count, glGenTextures(-1, names))
CALL(delete_textures_count, glDeleteTextures(-1, names))
CALL(tex_image_target, glTexImage2D(0x1234, 0, GL_RGBA, 2, 2, 0, GL_RGBA,
                                    GL_UNSIGNED_BYTE, pixels))
CALL(tex_image_level, glTexImage2D(GL_TEXTURE_2D, -1, GL_RGBA, 2, 2, 0,
                                   GL_RGBA, GL_UNSIGNED_BYTE, pixels))
CALL(tex_image_width, glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 3, 2, 0,
                                   GL_RGBA, GL_UNSIGNED_BYTE, pixels))
CALL(tex_image_border, glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 1,
                                    GL_RGBA, GL_UNSIGNED_BYTE, pixels))
CALL(tex_image_internal, glTexImage2D(GL_TEXTURE_2D, 0, 0x1234, 2, 2, 0,
                                      GL_RGBA, GL_UNSIGNED_BYTE, pixels))
CALL(tex_image_mismatch, glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 2, 2, 0,
                                      GL_RGBA, GL_UNSIGNED_BYTE, pixels))
CALL(tex_image_type, glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 0,
                                  GL_RGBA, 0x1234, pixels))
CALL(tex_image_packing, glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 2, 2, 0,
                                     GL_RGBA, GL_UNSIGNED_SHORT_5_6_5, pixels))
CALL(tex_parameter_target, glTexParameteri(0x1234, GL_TEXTURE_MIN_FILTER,
                                           GL_LINEAR))
CALL(tex_parameter_pname, glTexParameteri(GL_TEXTURE_2D, 0x1234, GL_LINEAR))
CALL(tex_parameter_value,
     glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, 0x1234))
CALL(tex_env_target, glTexEnvi(0x1234, GL_TEXTURE_ENV_MODE, GL_REPLACE))
CALL(tex_env_pname, glTexEnvi(GL_TEXTURE_ENV, 0x1234, GL_REPLACE))
CALL(tex_env_mode, glTexEnvi(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, 0x1234))
CALL(tex_env_scale, glTexEnvx(GL_TEXTURE_ENV, GL_RGB_SCALE, 3 * ONE))
CALL(active_texture_bad, glActiveTexture(GL_TEXTURE0 + 32))
/* Section 3.8: fog. */
CALL(fog_pname, glFogx(0x1234, 0))
CALL(fog_mode, glFogx(GL_FOG_MODE, 0x1234))
CALL(fog_density, glFogx(GL_FOG_DENSITY, -ONE))
/* Chapter 4: per-fragment operations and the framebuffer. */
CALL(scissor_negative, glScissor(0, 0, 4, -1))
CALL(alpha_func_bad, glAlphaFuncx(0x1234, 0))
CALL(stencil_func_bad, glStencilFunc(0x1234, 0, 0xff))
CALL(stencil_op_bad, glStencilOp(GL_KEEP, 0x1234, GL_KEEP))
CALL(depth_func_bad, glDepthFunc(0x1234))
CALL(blend_func_src, glBlendFunc(0x1234, GL_ZERO))
CALL(blend_func_dst, glBlendFunc(GL_ONE, GL_SRC_ALPHA_SATURATE))
CALL(logic_op_bad, glLogicOp(0x1234))
CALL(clear_bits, glClear(0x8000))
/* Chapter 5: hints. */
CALL(hint_target, glHint(0x1234, GL_NICEST))
CALL(hint_mode, glHint(GL_FOG_HINT, 0x1234))

static const struct error_case cases[] = {
#define C(name, want) {#name, call_##name, want}
    C(enable_bad, GL_INVALID_ENUM),
    C(disable_bad, GL_INVALID_ENUM),
    C(is_enabled_bad, GL_INVALID_ENUM),
    C(get_integer_bad, GL_INVALID_ENUM),
    C(get_fixed_bad, GL_INVALID_ENUM),
    C(get_boolean_bad, GL_INVALID_ENUM),
    C(get_string_bad, GL_INVALID_ENUM),
    C(enable_client_state_bad, GL_INVALID_ENUM),
    C(vertex_pointer_size, GL_INVALID_VALUE),
    C(vertex_pointer_type, GL_INVALID_ENUM),
    C(vertex_pointer_stride, GL_INVALID_VALUE),
    C(color_pointer_size, GL_INVALID_VALUE),
    C(color_pointer_type, GL_INVALID_ENUM),
    C(normal_pointer_type, GL_INVALID_ENUM),
    C(tex_coord_pointer_size, GL_INVALID_VALUE),
    C(client_active_texture_bad, GL_INVALID_ENUM),
    C(draw_arrays_mode, GL_INVALID_ENUM),
    C(draw_arrays_count, GL_INVALID_VALUE),
    C(draw_elements_mode, GL_INVALID_ENUM),
    C(draw_elements_count, GL_INVALID_VALUE),
    C(draw_elements_type, GL_INVALID_ENUM),
    C(bind_buffer_target, GL_INVALID_ENUM),
    C(gen_buffers_count, GL_INVALID_VALUE),
    C(delete_buffers_count, GL_INVALID_VALUE),
    C(buffer_data_size, GL_INVALID_VALUE),
    C(buffer_data_usage, GL_INVALID_ENUM),
    C(buffer_sub_data_range, GL_INVALID_VALUE),
    C(viewport_negative, GL_INVALID_VALUE),
    C(matrix_mode_bad, GL_INVALID_ENUM),
    C(frustum_near, GL_INVALID_VALUE),
    C(frustum_sides, GL_INVALID_VALUE),
    C(frustum_depth, GL_INVALID_VALUE),
    C(ortho_sides, GL_INVALID_VALUE),
    C(ortho_depth, GL_INVALID_VALUE),
    C(pop_underflow, GL_STACK_UNDERFLOW),
    C(clip_plane_bad, GL_INVALID_ENUM),
    C(light_bad, GL_INVALID_ENUM),
    C(light_pname, GL_INVALID_ENUM),
    C(light_vector_pname, GL_INVALID_ENUM),
    C(light_spot_exponent, GL_INVALID_VALUE),
    C(light_spot_cutoff, GL_INVALID_VALUE),
    C(light_attenuation, GL_INVALID_VALUE),
    C(material_face, GL_INVALID_ENUM),
    C(material_pname, GL_INVALID_ENUM),
    C(material_vector_pname, GL_INVALID_ENUM),
    C(material_shininess, GL_INVALID_VALUE),
    C(light_model_pname, GL_INVALID_ENUM),
    C(shade_model_bad, GL_INVALID_ENUM),
    C(point_size_zero, GL_INVALID_VALUE),
    C(point_size_negative, GL_INVALID_VALUE),
    C(line_width_zero, GL_INVALID_VALUE),
    C(cull_face_bad, GL_INVALID_ENUM),
    C(front_face_bad, GL_INVALID_ENUM),
    C(pixel_store_pname, GL_INVALID_ENUM),
    C(pixel_store_value, GL_INVALID_VALUE),
    C(read_pixels_format, GL_INVALID_ENUM),
    C(read_pixels_type, GL_INVALID_ENUM),
    C(read_pixels_size, GL_INVALID_VALUE),
    C(bind_texture_target, GL_INVALID_ENUM),
    C(gen_textures_count, GL_INVALID_VALUE),
    C(delete_textures_count, GL_INVALID_VALUE),
    C(tex_image_target, GL_INVALID_ENUM),
    C(tex_image_level, GL_INVALID_VALUE),
    C(tex_image_width, GL_INVALID_VALUE),
    C(tex_image_border, GL_INVALID_VALUE),
    C(tex_image_internal, GL_INVALID_VALUE),
    C(tex_image_mismatch, GL_INVALID_OPERATION),
    C(tex_image_type, GL_INVALID_ENUM),
    C(tex_image_packing, GL_INVALID_OPERATION),
    C(tex_parameter_target, GL_INVALID_ENUM),
    C(tex_parameter_pname, GL_INVALID_ENUM),
    C(tex_parameter_value, GL_INVALID_ENUM),
    C(tex_env_target, GL_INVALID_ENUM),
    C(tex_env_pname, GL_INVALID_ENUM),
    C(tex_env_mode, GL_INVALID_ENUM),
    C(tex_env_scale, GL_INVALID_VALUE),
    C(active_texture_bad, GL_INVALID_ENUM),
    C(fog_pname, GL_INVALID_ENUM),
    C(fog_mode, GL_INVALID_ENUM),
    C(fog_density, GL_INVALID_VALUE),
    C(scissor_negative, GL_INVALID_VALUE),
    C(alpha_func_bad, GL_INVALID_ENUM),
    C(stencil_func_bad, GL_INVALID_ENUM),
    C(stencil_op_bad, GL_INVALID_ENUM),
    C(depth_func_bad, GL_INVALID_ENUM),
    C(blend_func_src, GL_INVALID_ENUM),
    C(blend_func_dst, GL_INVALID_ENUM),
    C(logic_op_bad, GL_INVALID_ENUM),
    C(clear_bits, GL_INVALID_VALUE),
    C(hint_target, GL_INVALID_ENUM),
    C(hint_mode, GL_INVALID_ENUM),
#undef C
};

/* One case: the error it gives, once. */
static void run(const struct error_case *c) {
  while (glGetError() != GL_NO_ERROR)
    ;
  c->call();
  GLenum e = glGetError();
  GLenum after = glGetError();
  if (e != c->want)
    conform_fail("errors", c->name, "%04x, not %04x", e, c->want);
  else if (after != GL_NO_ERROR)
    conform_fail("errors", c->name, "a second error %04x after it", after);
  else
    conform_pass("errors", c->name);
}

/* A push onto the full projection stack: STACK_OVERFLOW, and the stack
 * as it was. */
static void overflow(void) {
  GLint most = 0, depth = 0;
  glGetIntegerv(GL_MAX_PROJECTION_STACK_DEPTH, &most);
  glMatrixMode(GL_PROJECTION);
  for (int k = 1; k < most; k++)
    glPushMatrix();
  while (glGetError() != GL_NO_ERROR)
    ;
  glPushMatrix();
  GLenum e = glGetError();
  glGetIntegerv(GL_PROJECTION_STACK_DEPTH, &depth);
  for (int k = 1; k < most; k++)
    glPopMatrix();
  glMatrixMode(GL_MODELVIEW);
  while (glGetError() != GL_NO_ERROR)
    ;
  if (e != GL_STACK_OVERFLOW)
    conform_fail("errors", "push_overflow", "%04x, not %04x", e,
                 GL_STACK_OVERFLOW);
  else if (depth != most)
    conform_fail("errors", "push_overflow", "the depth is %d, not %d", depth,
                 most);
  else
    conform_pass("errors", "push_overflow");
}

/* Section 2.5: an erroneous command changes nothing. Each sets a state
 * variable to a value it does not hold, by a call that fails, and reads it
 * back. */
static void unchanged(const char *name, void (*bad)(void), GLenum pname,
                      GLint want) {
  while (glGetError() != GL_NO_ERROR)
    ;
  bad();
  glGetError();
  GLint got[4] = {0x7eadbeef};
  glGetIntegerv(pname, got);
  if (got[0] == want)
    conform_pass("errors", name);
  else
    conform_fail("errors", name, "%d, not %d as it was", got[0], want);
}

static void bad_viewport(void) { glViewport(5, 5, -1, -1); }
static void bad_scissor(void) { glScissor(5, 5, -1, 4); }
static void bad_matrix_mode(void) { glMatrixMode(GL_TEXTURE + 1); }
static void bad_depth_func(void) { glDepthFunc(GL_ALWAYS + 1); }
static void bad_stencil_func(void) { glStencilFunc(GL_ALWAYS + 1, 7, 0x0f); }
static void bad_pack(void) { glPixelStorei(GL_PACK_ALIGNMENT, 3); }
static void bad_pop(void) { glPopMatrix(); }

/* Section 2.5: errors are kept until GetError reads them, each once, and
 * GetError then gives NO_ERROR. */
static void kept(void) {
  while (glGetError() != GL_NO_ERROR)
    ;
  glEnable(0x1234);
  glViewport(0, 0, -1, -1);
  GLenum seen[8];
  int n = 0;
  while (n < 8 && (seen[n] = glGetError()) != GL_NO_ERROR)
    n++;
  /* The first is kept: GL has a flag a kind, and an implementation that
   * keeps one flag keeps the first error. */
  if (n == 0 || seen[0] != GL_INVALID_ENUM)
    conform_fail("errors", "first_error_kept", "%04x first", n ? seen[0] : 0);
  else if (n > 6)
    conform_fail("errors", "first_error_kept", "%d errors, more than flags",
                 n);
  else
    conform_pass("errors", "first_error_kept");
}

void conform_errors(void) {
  for (unsigned k = 0; k < sizeof cases / sizeof cases[0]; k++)
    run(&cases[k]);
  overflow();
  unchanged("viewport_unchanged", bad_viewport, GL_VIEWPORT, 0);
  unchanged("scissor_unchanged", bad_scissor, GL_SCISSOR_BOX, 0);
  unchanged("matrix_mode_unchanged", bad_matrix_mode, GL_MATRIX_MODE,
            GL_MODELVIEW);
  unchanged("depth_func_unchanged", bad_depth_func, GL_DEPTH_FUNC, GL_LESS);
  unchanged("stencil_func_unchanged", bad_stencil_func, GL_STENCIL_FUNC,
            GL_ALWAYS);
  unchanged("pack_alignment_unchanged", bad_pack, GL_PACK_ALIGNMENT, 4);
  unchanged("pop_unchanged", bad_pop, GL_MODELVIEW_STACK_DEPTH, 1);
  kept();
}
