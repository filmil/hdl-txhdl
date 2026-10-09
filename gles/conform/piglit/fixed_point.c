/*
 * Copyright © 2013 Intel Corporation
 *
 * Permission is hereby granted, free of charge, to any person obtaining a
 * copy of this software and associated documentation files (the "Software"),
 * to deal in the Software without restriction, including without limitation
 * the rights to use, copy, modify, merge, publish, distribute, sublicense,
 * and/or sell copies of the Software, and to permit persons to whom the
 * Software is furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice (including the next
 * paragraph) shall be included in all copies or substantial portions of the
 * Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL
 * THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
 * IN THE SOFTWARE.
 */

/**
 * @file
 * @brief Test GL_FIXED with attribute arrays in OpenGL ES 1.1.
 *
 * This test paints the window's left half green and window's right half dark
 * blue. It uses the GL_FIXED data type for glVertexPointer and
 * glColorPointer.
 *
 * This tests Mesa commit 7a9f4d3e for Intel gen4+.
 */

#include "piglit-util-gl.h"

PIGLIT_GL_TEST_CONFIG_BEGIN

	config.supports_gl_es_version = 11;
	config.window_visual = PIGLIT_GL_VISUAL_RGBA | PIGLIT_GL_VISUAL_DOUBLE;
	config.khr_no_error_support = PIGLIT_NO_ERRORS;

PIGLIT_GL_TEST_CONFIG_END

/* From the GL_OES_fixed_point spec, GL_FIXED represents a
 * "signed 2's complement S15.16 scaled integer".
 */
#define ONE  0x00010000
#define HALF 0x00008000

/* Vertices for the window's left half. */
static const GLfixed left_vertices[] = {
	-ONE, -ONE,
	   0, -ONE,
	   0, +ONE,
	-ONE, +ONE,
};

/* Vertices for the window's right half. */
static const GLfixed right_vertices[] = {
	   0, -ONE,
	+ONE, -ONE,
	+ONE, +ONE,
	   0, +ONE,
};

/* Green, color of the window's left half. */
static const GLfloat left_color_float[4] = {0, 1, 0, 1};
static const GLfixed left_colors_fixed[] = {
	0, ONE, 0, ONE,
	0, ONE, 0, ONE,
	0, ONE, 0, ONE,
	0, ONE, 0, ONE,
};

/* Dark blue, color of the window's right half. */
static const GLfloat right_color_float[4] = {0, 0, 0.5, 1};
static const GLfixed right_colors_fixed[] = {
	0, 0, HALF, ONE,
	0, 0, HALF, ONE,
	0, 0, HALF, ONE,
	0, 0, HALF, ONE,
};

enum piglit_result
piglit_display(void)
{
	bool pass = true;

	glClearColor(0, 0, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);

	glEnableClientState(GL_VERTEX_ARRAY);
	glEnableClientState(GL_COLOR_ARRAY);

	/* Paint the window's left half. */
	glVertexPointer(2, GL_FIXED, 0, left_vertices);
	glColorPointer(4, GL_FIXED, 0, left_colors_fixed);
	glDrawArrays(GL_TRIANGLE_FAN, 0, 4);

	/* Paint the window's right half.*/
	glVertexPointer(2, GL_FIXED, 0, right_vertices);
	glColorPointer(4, GL_FIXED, 0, right_colors_fixed);
	glDrawArrays(GL_TRIANGLE_FAN, 0, 4);

	/* Probe the window's left half. */
	pass &= piglit_probe_rect_rgba(
			0, 0,
			piglit_width / 2, piglit_height / 2,
			left_color_float);

	/* Probe the window's right half. */
	pass &= piglit_probe_rect_rgba(
			(piglit_width + 1) / 2, (piglit_height + 1) / 2,
			(piglit_width - 1) / 2,  (piglit_height - 1) / 2,
			right_color_float);

	piglit_present_results();

	return pass ? PIGLIT_PASS : PIGLIT_FAIL;
}

void
piglit_init(int argc, char **argv)
{
	piglit_require_extension("GL_OES_fixed_point");
}
