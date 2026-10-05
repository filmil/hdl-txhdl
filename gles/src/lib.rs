// SPDX-License-Identifier: Apache-2.0
//! GL ES 1.1 Common-Lite for Razboj (`docs/gles.md`, issue 1159, the
//! first step of #995): 16.16 fixed point, the matrices GL's calls make,
//! and GL's enumerants. No standard library, so that a program on
//! Vreteno links it.
#![cfg_attr(not(test), no_std)]

pub mod fixed;
pub mod gl;
pub mod matrix;
