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
//!
//! A VHDL entity goes the same way with nvc as the engine, run as a
//! child process, since nvc is a program and not a library: the tool
//! writes a testbench around the entity that reads a line per step
//! from its standard input, a command and the inputs as hex, applies
//! them, settles or pulses the clock, and writes the outputs back as
//! one line; `Cosim` runs the nvc test script the build made of that
//! testbench and speaks the lines. The same generated unit drives
//! either engine, since both answer to set, get, settle and edge.
//! `vhdl_unit()` builds the testbench, the script and the crate.
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

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

/// An entity under nvc, a child process behind the test script the
/// build made of its co-run testbench. A step is a line each way: a
/// settle sends `S` and the inputs as hex and reads the outputs, an
/// edge sends `E` and reads them again; the inputs are kept between
/// the two, and `get` reads what the last line said.
pub struct Cosim {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
    /// Each input port's index and width, in the line's order, and
    /// its value as last set.
    inputs: Vec<(u32, usize, u128)>,
    /// Each output port's index and its value as last read.
    outputs: Vec<(u32, u128)>,
}

impl Cosim {
    /// Start the script, named by its path under the main repository
    /// in the runfiles, which are beside the binary or where
    /// `RUNFILES_DIR` says; the script expects to run in the runfiles'
    /// main directory and to be told a scratch directory as a test
    /// would.
    pub fn new(
        script: &str,
        ins: &[(u32, usize)],
        outs: &[(u32, usize)],
    ) -> Self {
        let runfiles = match std::env::var_os("RUNFILES_DIR") {
            Some(d) => std::path::PathBuf::from(d),
            None => {
                let exe = std::env::current_exe().expect("the binary's path");
                let mut r = exe.into_os_string();
                r.push(".runfiles");
                std::path::PathBuf::from(r)
            }
        };
        let root = runfiles.join("_main");
        let script = root.join(script);
        let scratch = std::env::temp_dir()
            .join(format!("txhdl-cosim-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("a scratch directory");
        let mut child = Command::new(&script)
            .current_dir(&root)
            .env("RUNFILES_DIR", &runfiles)
            .env_remove("RUNFILES_MANIFEST_FILE")
            .env("TEST_TMPDIR", &scratch)
            .env("TEST_UNDECLARED_OUTPUTS_DIR", &scratch)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("nvc under its script");
        let to = child.stdin.take().expect("stdin");
        let from = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            to,
            from,
            inputs: ins.iter().map(|&(i, w)| (i, w, 0)).collect(),
            outputs: outs.iter().map(|&(i, _)| (i, 0)).collect(),
        }
    }

    pub fn set(&mut self, port: u32, v: u128) {
        if let Some(e) = self.inputs.iter_mut().find(|e| e.0 == port) {
            e.2 = v;
        }
    }

    pub fn get(&self, port: u32) -> u128 {
        self.outputs
            .iter()
            .find(|e| e.0 == port)
            .map(|e| e.1)
            .unwrap_or(0)
    }

    pub fn eval(&mut self) {
        self.step('S');
    }

    pub fn edge(&mut self) {
        self.step('E');
    }

    fn step(&mut self, what: char) {
        let mut line = String::from(what);
        for &(_, w, v) in &self.inputs {
            if w == 1 {
                line.push_str(if v & 1 == 1 { " 1" } else { " 0" });
            } else {
                line.push_str(&format!(" {:0>1$x}", v, w.div_ceil(4)));
            }
        }
        line.push('\n');
        self.to.write_all(line.as_bytes()).expect("write to nvc");
        self.to.flush().expect("flush to nvc");
        let mut reply = String::new();
        self.from.read_line(&mut reply).expect("read from nvc");
        assert!(!reply.is_empty(), "nvc ended without answering");
        for (slot, tok) in self.outputs.iter_mut().zip(reply.split_whitespace())
        {
            slot.1 = u128::from_str_radix(tok, 16).unwrap_or(0);
        }
    }
}

impl Drop for Cosim {
    fn drop(&mut self) {
        let _ = self.to.write_all(b"Q\n");
        let _ = self.to.flush();
        let _ = self.child.wait();
    }
}
