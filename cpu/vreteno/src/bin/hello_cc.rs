// SPDX-License-Identifier: Apache-2.0
//! Run the C++ program on the core, and check that it says what it
//! was written to say.
//!
//! `cpu/vreteno/cpp/hello.cc` is compiled for Vreteno by the GNU
//! bare-metal RISC-V toolchain the build fetches, against the
//! `libstdc++` that toolchain carries for this exact core, and turned
//! into an image by the same tool the Rust programs go through. The
//! machine is the same one too: what changes between the two is the
//! language and nothing else.
use vreteno32::run::{expect, run};

/// What the program is written to print. The number is worked out by
/// the compiler, not by the core: it is the sum of the squares of one
/// to eight.
const EXPECTED: &str = "hello from c++\nthe squares to eight sum to 204\n";

fn main() {
    let ran = run(hello_cc_program::TEXT, hello_cc_program::DATA, 8000);
    expect("the C++ program", &ran, EXPECTED);
    println!("C++, compiled for Vreteno, ran on it and said its line.");
}

/// The same run, as a test.
#[cfg(test)]
mod tests {
    use super::EXPECTED;
    use vreteno32::run::{expect, run};

    #[test]
    fn cpp_runs_on_vreteno() {
        let ran = run(hello_cc_program::TEXT, hello_cc_program::DATA, 8000);
        expect("the C++ program", &ran, EXPECTED);
    }
}
