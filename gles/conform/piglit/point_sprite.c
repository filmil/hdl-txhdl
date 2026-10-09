/*
 * Copyright © 2009 Intel Corporation
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
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
 * DEALINGS IN THE SOFTWARE.
 */

/**
 * This test draws a point sprite with a checkerboard texture and tests
 * whether the correct colors were drawn using piglit_probe_pixel_rgb.
 *
 * \author Ben Holmes
 */

#include "piglit-util-gl.h"

#define BOX_SIZE 64
#define TEST_COLS 6
#define TEST_ROWS 2

PIGLIT_GL_TEST_CONFIG_BEGIN

	config.supports_gl_compat_version = 10;
	config.supports_gl_es_version = 10;

	config.window_width = 1+((BOX_SIZE+1)*TEST_COLS);
	config.window_height = 1+((BOX_SIZE+1)*TEST_ROWS);
	config.window_visual = PIGLIT_GL_VISUAL_DOUBLE | PIGLIT_GL_VISUAL_RGB;
	config.khr_no_error_support = PIGLIT_NO_ERRORS;

PIGLIT_GL_TEST_CONFIG_END

static float maxSize = 0.0f;
static GLuint tex;
static const GLfloat black[4] = {0.0, 0.0, 0.0, 1.0};
static const GLfloat white[4] = {1.0, 1.0, 1.0, 1.0};

void
piglit_init(int argc, char **argv)
{
	GLfloat realMaxSize;

	(void) argc;
	(void) argv;

#ifdef PIGLIT_USE_OPENGL
	piglit_require_extension("GL_ARB_point_sprite");
#else
	piglit_require_extension("GL_OES_point_sprite");
#endif

	piglit_ortho_projection(piglit_width, piglit_height, GL_FALSE);

	glEnable(GL_TEXTURE_2D);
	glEnable(GL_POINT_SPRITE);

	glGetFloatv(GL_POINT_SIZE_MAX, &realMaxSize);
	maxSize = (realMaxSize > BOX_SIZE) ? BOX_SIZE : realMaxSize;

	glClearColor(0.2, 0.2, 0.2, 1.0);
	glColor4f(1.0, 1.0, 1.0, 1.0);

	tex = piglit_checkerboard_texture(0, 0, 2, 2, 1, 1, black, white);
	glTexEnvi(GL_POINT_SPRITE, GL_COORD_REPLACE, GL_TRUE);

	if (!piglit_automatic)
		printf("Maximum point size is %f, using %f\n", 
		       realMaxSize, maxSize);
}

enum piglit_result
piglit_display(void)
{
#ifdef PIGLIT_USE_OPENGL
	const unsigned num_rows = (piglit_get_gl_version() >= 20) ? 2 : 1;
#else
	const unsigned num_rows = 1;
#endif
	static const GLenum origins[2] = { GL_UPPER_LEFT, GL_LOWER_LEFT	};
	GLboolean pass = GL_TRUE;
	unsigned i;
	unsigned j;

	glClear(GL_COLOR_BUFFER_BIT);
	glBindTexture(GL_TEXTURE_2D, tex);

	for (i = 0; i < num_rows; i++) {
		const float y = 1 + (BOX_SIZE / 2) + (i * (BOX_SIZE + 1));
		const float *const upper_left = (origins[i] == GL_UPPER_LEFT)
			? black : white;
		const float *const lower_left = (origins[i] == GL_UPPER_LEFT)
			? white : black;

#ifdef PIGLIT_USE_OPENGL
		/* OpenGL version must be at least 2.0 to support modifying
		 * GL_POINT_SPRITE_COORD_ORIGIN.
		 */
		if (piglit_get_gl_version() >= 20)
			glPointParameteri(GL_POINT_SPRITE_COORD_ORIGIN,
					  origins[i]);
#endif

		for (j = 0; j < TEST_COLS; j++) {
			const float x = 1 + (BOX_SIZE / 2) 
				+ (j * (BOX_SIZE + 1));
			const float size = maxSize / (float) (1 << j);

			/* If the point size is too small, there won't be
			 * enough pixels drawn for the tests (below).
			 */
			if (size < 2.0)
				continue;

			glPointSize(size - 0.2);

			/* Vertex arrays are overkill for this case,
			 * but they are necessary for OpenGL ES 1.x.
			 */
			GLfloat tcp[] = { 1.5, 1.5 };
			GLfloat vp[] = { x, y };

			glEnableClientState(GL_VERTEX_ARRAY);
			glEnableClientState(GL_TEXTURE_COORD_ARRAY);

			glTexCoordPointer(2, GL_FLOAT, 0, tcp);
			glVertexPointer(2, GL_FLOAT, 0, vp);
			glDrawArrays(GL_POINTS, 0, 1);

			glDisableClientState(GL_VERTEX_ARRAY);
			glDisableClientState(GL_TEXTURE_COORD_ARRAY);

			if (!piglit_probe_pixel_rgb(x - (size / 4),
						    y + (size / 4),
						    upper_left)
			    || !piglit_probe_pixel_rgb(x - (size / 4),
						       y - (size / 4),
						       lower_left)
			    || !piglit_probe_pixel_rgb(x + (size / 4),
						       y + (size / 4),
						       lower_left)
			    || !piglit_probe_pixel_rgb(x + (size / 4),
						       y - (size / 4),
						       upper_left)) {
				if (!piglit_automatic)
					printf("  size = %.3f, "
					       "origin = %s left\n",
					       size,
					       (origins[i] == GL_UPPER_LEFT)
					       ? "upper" : "lower");
				pass = GL_FALSE;
			}
		}
	}

	piglit_present_results();

	return pass ? PIGLIT_PASS : PIGLIT_FAIL;
}
