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
 * \file oes_matrix_get-api.c
 * Test the queries added by GL_OES_matrix_get.
 */

#include "piglit-util-gl.h"

PIGLIT_GL_TEST_CONFIG_BEGIN

	config.supports_gl_es_version = 11;

PIGLIT_GL_TEST_CONFIG_END

enum piglit_result
piglit_display(void)
{
	/* UNREACHED */
	return PIGLIT_FAIL;
}

void
piglit_init(int argc, char **argv)
{
	static const struct {
		GLenum set;
		GLenum get;
	} test_vectors[] = {
		{ GL_MODELVIEW, GL_MODELVIEW_MATRIX_FLOAT_AS_INT_BITS_OES },
		{ GL_PROJECTION, GL_PROJECTION_MATRIX_FLOAT_AS_INT_BITS_OES },
		{ GL_TEXTURE, GL_TEXTURE_MATRIX_FLOAT_AS_INT_BITS_OES },
	};

	static const float m[] = {
		10., 11., 12., 13.,
		14., 15., 16., 17.,
		18., 19., 20., 21.,
		22., 23., 24., 25.
	};

	float got[ARRAY_SIZE(m)];
	unsigned i;
	bool pass = true;

	piglit_require_extension("GL_OES_matrix_get");

	for (i = 0; i < ARRAY_SIZE(test_vectors); i++) {
		glMatrixMode(test_vectors[i].set);
		glLoadMatrixf(m);

		memset(got, 0, sizeof(got));
		glGetIntegerv(test_vectors[i].get, (GLint *) got);
                pass = piglit_check_gl_error(GL_NO_ERROR) && pass;

		if (memcmp(got, m, sizeof(m)) != 0) {
			unsigned j;

			fprintf(stderr, "Data mismatch for %s. Got:\n",
				piglit_get_gl_enum_name(test_vectors[i].set));

			for (j = 0; j < ARRAY_SIZE(got); j++) {
				fprintf(stderr, "%f%s",
					got[j], (j % 4) == 3 ? "\n" : ", ");
			}
			
			pass = false;
		}
	}

	piglit_report_result(pass ? PIGLIT_PASS : PIGLIT_FAIL);
}
