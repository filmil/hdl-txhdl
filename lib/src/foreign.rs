// SPDX-License-Identifier: Apache-2.0
//! A foreign module as a unit. A Verilog module is compiled by
//! Verilator into a C++ model; a C shim over it, written by
//! `//tools/vshim` from the module's ports, gives the model a handle,
//! sets and reads its ports by index, settles it and pulses its clock;
//! and a unit, written by the same tool, drives the shim once per
//! rising edge: the inputs in as the edge left them, a settle, the
//! transfers the module agrees to on its channels, the edge, and the
//! outputs out. So the module is a registered unit like any other,
//! goes first in the join order like one, and the rest of the design
//! is none the wiser. `verilog_unit()` in `lib/foreign.bzl` builds
//! the three from the one file.
use std::ffi::c_void;

/// The shim's functions, one set per module.
pub struct Shim {
    pub new: unsafe extern "C" fn() -> *mut c_void,
    pub free: unsafe extern "C" fn(*mut c_void),
    pub eval: unsafe extern "C" fn(*mut c_void),
    pub edge: unsafe extern "C" fn(*mut c_void),
    pub set: unsafe extern "C" fn(*mut c_void, u32, *const u32, u32),
    pub get: unsafe extern "C" fn(*mut c_void, u32, *mut u32, u32),
}

/// A model under its shim: ports by index, values up to 128 bits as
/// four words.
pub struct Model {
    shim: Shim,
    m: *mut c_void,
}

impl Model {
    pub fn new(shim: Shim) -> Self {
        let m = unsafe { (shim.new)() };
        Self { shim, m }
    }

    /// Drive an input port.
    pub fn set(&self, port: u32, v: u128) {
        let w = [
            v as u32,
            (v >> 32) as u32,
            (v >> 64) as u32,
            (v >> 96) as u32,
        ];
        unsafe { (self.shim.set)(self.m, port, w.as_ptr(), 4) }
    }

    /// Read a port.
    pub fn get(&self, port: u32) -> u128 {
        let mut w = [0u32; 4];
        unsafe { (self.shim.get)(self.m, port, w.as_mut_ptr(), 4) }
        w[0] as u128
            | (w[1] as u128) << 32
            | (w[2] as u128) << 64
            | (w[3] as u128) << 96
    }

    /// Settle the combinational logic on the inputs as they stand.
    pub fn eval(&self) {
        unsafe { (self.shim.eval)(self.m) }
    }

    /// One clock edge: the clock up, settled, and down, settled.
    pub fn edge(&self) {
        unsafe { (self.shim.edge)(self.m) }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { (self.shim.free)(self.m) }
    }
}
