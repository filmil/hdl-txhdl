// SPDX-License-Identifier: Apache-2.0
//! The GL ES 1.1 enumerants this library takes, with the values
//! Khronos's `GLES/gl.h` gives them, so that a C program's calls mean
//! the same once the C ABI is in front (#995).

pub const NO_ERROR: u32 = 0;
pub const INVALID_ENUM: u32 = 0x0500;
pub const INVALID_VALUE: u32 = 0x0501;
pub const INVALID_OPERATION: u32 = 0x0502;
pub const STACK_OVERFLOW: u32 = 0x0503;
pub const STACK_UNDERFLOW: u32 = 0x0504;
pub const OUT_OF_MEMORY: u32 = 0x0505;

pub const POINTS: u32 = 0x0000;
pub const LINES: u32 = 0x0001;
pub const LINE_LOOP: u32 = 0x0002;
pub const LINE_STRIP: u32 = 0x0003;
pub const TRIANGLES: u32 = 0x0004;
pub const TRIANGLE_STRIP: u32 = 0x0005;
pub const TRIANGLE_FAN: u32 = 0x0006;

pub const FRONT: u32 = 0x0404;
pub const BACK: u32 = 0x0405;
pub const FRONT_AND_BACK: u32 = 0x0408;
pub const CW: u32 = 0x0900;
pub const CCW: u32 = 0x0901;

pub const CULL_FACE: u32 = 0x0B44;
pub const CLIP_PLANE0: u32 = 0x3000;

pub const MODELVIEW: u32 = 0x1700;
pub const PROJECTION: u32 = 0x1701;

pub const FLAT: u32 = 0x1D00;
pub const SMOOTH: u32 = 0x1D01;

pub const COLOR_BUFFER_BIT: u32 = 0x0000_4000;
pub const DEPTH_BUFFER_BIT: u32 = 0x0000_0100;

/// Depth (#1273): the test's switch, and its comparisons, whose order
/// from `NEVER` is Razboj's (#992).
pub const DEPTH_TEST: u32 = 0x0B71;
pub const POLYGON_OFFSET_FILL: u32 = 0x8037;
pub const POLYGON_OFFSET_FACTOR: u32 = 0x8038;
pub const POLYGON_OFFSET_UNITS: u32 = 0x2A00;
pub const DITHER: u32 = 0x0BD0;
/// The logic operations (#998): the switch, the query, and GL's sixteen
/// from `GL_CLEAR` to `GL_SET`, of which these are the ends and the one
/// GL starts with.
pub const COLOR_LOGIC_OP: u32 = 0x0BF2;
pub const LOGIC_OP_MODE: u32 = 0x0BF0;
pub const CLEAR: u32 = 0x1500;
pub const COPY: u32 = 0x1503;
pub const SET: u32 = 0x150F;
pub const SCISSOR_TEST: u32 = 0x0C11;
/// The hints (#1490): GL ES 1.1's targets, and the three modes.
pub const PERSPECTIVE_CORRECTION_HINT: u32 = 0x0C50;
pub const POINT_SMOOTH_HINT: u32 = 0x0C51;
pub const LINE_SMOOTH_HINT: u32 = 0x0C52;
pub const FOG_HINT: u32 = 0x0C54;
pub const GENERATE_MIPMAP_HINT: u32 = 0x8192;
pub const DONT_CARE: u32 = 0x1100;
pub const FASTEST: u32 = 0x1101;
pub const NICEST: u32 = 0x1102;
pub const NEVER: u32 = 0x0200;
pub const LESS: u32 = 0x0201;
pub const EQUAL: u32 = 0x0202;
pub const LEQUAL: u32 = 0x0203;
pub const GREATER: u32 = 0x0204;
pub const NOTEQUAL: u32 = 0x0205;
pub const GEQUAL: u32 = 0x0206;
pub const ALWAYS: u32 = 0x0207;

/// The modelview stack's depth, the specification's minimum.
pub const MAX_MODELVIEW_STACK_DEPTH: usize = 16;
/// The projection stack's depth, the specification's minimum.
pub const MAX_PROJECTION_STACK_DEPTH: usize = 2;

pub const LIGHTING: u32 = 0x0B50;
pub const LIGHT0: u32 = 0x4000;
pub const NORMALIZE: u32 = 0x0BA1;
pub const RESCALE_NORMAL: u32 = 0x803A;
pub const COLOR_MATERIAL: u32 = 0x0B57;

pub const AMBIENT: u32 = 0x1200;
pub const DIFFUSE: u32 = 0x1201;
pub const SPECULAR: u32 = 0x1202;
pub const POSITION: u32 = 0x1203;
pub const SPOT_DIRECTION: u32 = 0x1204;
pub const SPOT_EXPONENT: u32 = 0x1205;
pub const SPOT_CUTOFF: u32 = 0x1206;
pub const CONSTANT_ATTENUATION: u32 = 0x1207;
pub const LINEAR_ATTENUATION: u32 = 0x1208;
pub const QUADRATIC_ATTENUATION: u32 = 0x1209;

pub const EMISSION: u32 = 0x1600;
pub const SHININESS: u32 = 0x1601;
pub const AMBIENT_AND_DIFFUSE: u32 = 0x1602;

pub const LIGHT_MODEL_TWO_SIDE: u32 = 0x0B52;
pub const LIGHT_MODEL_AMBIENT: u32 = 0x0B53;

/// How many lights there are, the specification's minimum.
pub const MAX_LIGHTS: usize = 8;

/// The largest point size and line width, in pixels: the top of
/// `ALIASED_POINT_SIZE_RANGE` and `ALIASED_LINE_WIDTH_RANGE`, whose
/// bottom is one (#994).
pub const MAX_SIZE: u32 = 64;

/// Blending and the alpha test (#993): the switches, and the factors of
/// `glBlendFunc`, GL's `ZERO` and `ONE`, then `SRC_COLOR` to
/// `SRC_ALPHA_SATURATE` in GL's order, which is Razboj's from two.
pub const BLEND: u32 = 0x0BE2;
pub const ALPHA_TEST: u32 = 0x0BC0;
pub const ZERO: u32 = 0x0000;
pub const ONE: u32 = 0x0001;
pub const SRC_COLOR: u32 = 0x0300;
pub const ONE_MINUS_SRC_COLOR: u32 = 0x0301;
pub const SRC_ALPHA: u32 = 0x0302;
pub const ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
pub const DST_ALPHA: u32 = 0x0304;
pub const ONE_MINUS_DST_ALPHA: u32 = 0x0305;
pub const DST_COLOR: u32 = 0x0306;
pub const ONE_MINUS_DST_COLOR: u32 = 0x0307;
pub const SRC_ALPHA_SATURATE: u32 = 0x0308;

/// Textures (#997): the target and its switch, the matrix, the client
/// array, the parameters and their values, the environment, and the
/// formats and types `glTexImage2D` takes.
pub const TEXTURE_2D: u32 = 0x0DE1;
pub const TEXTURE: u32 = 0x1702;
pub const TEXTURE_COORD_ARRAY: u32 = 0x8078;
pub const TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const TEXTURE_WRAP_S: u32 = 0x2802;
pub const TEXTURE_WRAP_T: u32 = 0x2803;
pub const GENERATE_MIPMAP: u32 = 0x8191;
pub const NEAREST: u32 = 0x2600;
pub const LINEAR: u32 = 0x2601;
pub const NEAREST_MIPMAP_NEAREST: u32 = 0x2700;
pub const LINEAR_MIPMAP_NEAREST: u32 = 0x2701;
pub const NEAREST_MIPMAP_LINEAR: u32 = 0x2702;
pub const LINEAR_MIPMAP_LINEAR: u32 = 0x2703;
pub const REPEAT: u32 = 0x2901;
pub const CLAMP_TO_EDGE: u32 = 0x812F;
pub const TEXTURE_ENV: u32 = 0x2300;
pub const TEXTURE_ENV_MODE: u32 = 0x2200;
/// Point sprites, `OES_point_sprite` (#998).
pub const POINT_SPRITE_OES: u32 = 0x8861;
pub const COORD_REPLACE_OES: u32 = 0x8862;
pub const TEXTURE_ENV_COLOR: u32 = 0x2201;
pub const MODULATE: u32 = 0x2100;
pub const DECAL: u32 = 0x2101;
pub const REPLACE: u32 = 0x1E01;
pub const ADD: u32 = 0x0104;
pub const ALPHA: u32 = 0x1906;
pub const RGB: u32 = 0x1907;
pub const RGBA: u32 = 0x1908;
pub const LUMINANCE: u32 = 0x1909;
pub const LUMINANCE_ALPHA: u32 = 0x190A;
pub const UNSIGNED_BYTE: u32 = 0x1401;
pub const UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
pub const UNSIGNED_SHORT_5_5_5_1: u32 = 0x8034;
pub const UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
/// The paletted formats of `OES_compressed_paletted_texture` (#998).
pub const PALETTE4_RGB8_OES: u32 = 0x8B90;
pub const PALETTE4_RGBA8_OES: u32 = 0x8B91;
pub const PALETTE4_R5_G6_B5_OES: u32 = 0x8B92;
pub const PALETTE4_RGBA4_OES: u32 = 0x8B93;
pub const PALETTE4_RGB5_A1_OES: u32 = 0x8B94;
pub const PALETTE8_RGB8_OES: u32 = 0x8B95;
pub const PALETTE8_RGBA8_OES: u32 = 0x8B96;
pub const PALETTE8_R5_G6_B5_OES: u32 = 0x8B97;
pub const PALETTE8_RGBA4_OES: u32 = 0x8B98;
pub const PALETTE8_RGB5_A1_OES: u32 = 0x8B99;

/// The largest texture's side, Razboj's: ten bits of coordinate.
pub const MAX_TEXTURE_SIZE: u32 = 1024;

/// The texture stack's depth, the specification's minimum.
pub const MAX_TEXTURE_STACK_DEPTH: usize = 2;
pub const UNPACK_ALIGNMENT: u32 = 0x0CF5;
