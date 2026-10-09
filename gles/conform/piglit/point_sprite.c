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

/* TxHDL (#999): this test's names in the suite, by the shim. */
#define PIGLIT_PORT point_sprite
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
	/* TxHDL (#999): ES 1.1's names, of OES_point_sprite. */
	glEnable(GL_POINT_SPRITE_OES);

	/* TxHDL (#999): the fixed-point forms, as Common-Lite has them. */
	{
		GLfixed x = 0;

		glGetFixedv(GL_POINT_SIZE_MAX, &x);
		realMaxSize = x / 65536.0f;
	}
	maxSize = (realMaxSize > BOX_SIZE) ? BOX_SIZE : realMaxSize;

	glClearColorx(13107, 13107, 13107, 65536);
	glColor4x(65536, 65536, 65536, 65536);

	tex = piglit_checkerboard_texture(0, 0, 2, 2, 1, 1, black, white);
	glTexEnvi(GL_POINT_SPRITE_OES, GL_COORD_REPLACE_OES, GL_TRUE);

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
	/* TxHDL (#999): how many points were drawn and probed. */
	unsigned drawn = 0;
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

			glPointSizex((GLfixed)((size - 0.2) * 65536));

			/* Vertex arrays are overkill for this case,
			 * but they are necessary for OpenGL ES 1.x.
			 */
			GLfixed tcp[] = { 98304, 98304 };
			GLfixed vp[] = { (GLfixed)(x * 65536), (GLfixed)(y * 65536) };

			glEnableClientState(GL_VERTEX_ARRAY);
			glEnableClientState(GL_TEXTURE_COORD_ARRAY);

			glTexCoordPointer(2, GL_FIXED, 0, tcp);
			glVertexPointer(2, GL_FIXED, 0, vp);
			drawn++;
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

	/* TxHDL (#999): a test that probed no point has tested nothing, as
	 * when GL_POINT_SIZE_MAX cannot be read. */
	if (drawn == 0) {
		printf("# No point drawn: the largest size is %f\n", maxSize);
		pass = GL_FALSE;
	}

	return pass ? PIGLIT_PASS : PIGLIT_FAIL;
}
