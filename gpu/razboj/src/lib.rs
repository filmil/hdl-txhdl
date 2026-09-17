// SPDX-License-Identifier: Apache-2.0
//! Razboj, a minimal GPU in TxHDL: a rasteriser that reads a display
//! list and writes pixels into memory over AXI.
//!
//! - [`op`]: what a display list entry says.
//! - [`raster`]: the rasteriser, a unit and an AXI host.
//! - [`fb`]: the framebuffer, a unit and an AXI peripheral.
//! - [`model`]: the same rasteriser written with loops, which the
//!   hardware is checked against.
//! - [`image`]: the framebuffer as a PPM file and as colour on a
//!   terminal.
pub mod dl;
pub mod fb;
pub mod image;
pub mod model;
pub mod op;
pub mod raster;
pub mod scene;
pub mod sim;
