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

/// The modelview stack's depth, the specification's minimum.
pub const MAX_MODELVIEW_STACK_DEPTH: usize = 16;
/// The projection stack's depth, the specification's minimum.
pub const MAX_PROJECTION_STACK_DEPTH: usize = 2;
