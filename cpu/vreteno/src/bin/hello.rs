// SPDX-License-Identifier: Apache-2.0
//! Run the Rust program on the core, and check that it says what it
//! was written to say.
//!
//! The program in `cpu/vreteno/rust/hello.rs` is compiled for Vreteno
//! itself, by the toolchain `MODULE.bazel` fetches, and turned into
//! an image by `//tools/elf2vreteno`. The machine it runs on is
//! `vreteno32::run`, the same one the C++ program runs on.
use vreteno32::run::{expect, run};

/// What the program is written to print.
const EXPECTED: &str = "hello from rust\n";

fn main() {
    let ran = run(hello_program::TEXT, hello_program::DATA, 4000);
    expect("the Rust program", &ran, EXPECTED);
    println!("Rust, compiled for Vreteno, ran on it and said its line.");
}

/// The same run, as a test, so that `bazel test //...` checks that
/// Rust compiled for the core still runs on it.
#[cfg(test)]
mod tests {
    use super::EXPECTED;
    use vreteno32::run::{expect, run};

    #[test]
    fn rust_runs_on_vreteno() {
        let ran = run(hello_program::TEXT, hello_program::DATA, 4000);
        expect("the Rust program", &ran, EXPECTED);
    }
}
