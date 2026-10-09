# piglit's ES 1 tests, ported

Four of piglit's OpenGL ES 1 tests, from the Mesa project's piglit (`https://gitlab.freedesktop.org/mesa/piglit`, commit `64271b07ff6437afe6cf97e74af3498f03f783d4`), each under the MIT licence its own header states.
They are the four of piglit's nine ES 1 programs that test what the GL library has (#999):

| File here | piglit's file |
|---|---|
| `fixed_point.c` | `tests/spec/oes_fixed_point/oes_fixed_point-attribute-arrays.c` |
| `matrix_get.c` | `tests/spec/oes_matrix_get/api.c` |
| `paletted.c` | `tests/spec/oes_compressed_paletted_texture/oes_compressed_paletted_texture-api.c` |
| `point_sprite.c` | `tests/spec/arb_point_sprite/checkerboard.c` |

The first commit that adds them adds them as piglit has them.
The changes since are marked in each file with `TxHDL (#999):`: piglit's float calls become their fixed-point forms, since the library is the Common-Lite profile, and the requirements of extensions that ES 1.1 makes core additions are dropped.
One more, marked `TxHDL (#1536):`, is a fix to piglit's own test: `paletted.c`'s buffer is 768 bytes, but it hands `glCompressedTexImage2D` images of up to 1313, so GL reads past it, and on the board that read runs off the core's data memory.
The buffer here holds the largest image and is zeroed.
The overrun is upstream's too, and #1536 tracks the report to piglit.
`piglit-util-gl.h` and `piglit.c` give the parts of piglit's utility library the four use, and run them within the conformance suite.
