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
