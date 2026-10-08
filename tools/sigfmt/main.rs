// SPDX-License-Identifier: Apache-2.0
//! Turns a signature written by the Sail reference model into the
//! assembler source a self-checking test includes.
//!
//! This is `process_signature_file` from the ACT4 framework
//! (`framework/src/act/sig_modify.py` in riscv-arch-test), for XLEN
//! 32, rewritten so that the build needs no Python. Each line of the
//! signature, one hexadecimal word, becomes a `.word` directive. Three
//! labels are placed as the framework places them: `sig_end_canary`
//! before the last word, and `final_sig_offset`, `final_trap_sig_offset`
//! and `trap_sigptr` after the words that hold their canaries.
//!
//! Usage: `sigfmt <signature> <results>`.
use std::fs;
use std::process::exit;

const TRAP_CANARY: &str = "d3a91f6c";
const FINAL_SIG_OFFSET_CANARY: &str = "4b8e2d17";
const FINAL_TRAP_OFFSET_CANARY: &str = "7a110ff5";

/// Formats the signature `sig`, as Sail writes it, as assembler
/// source, and returns the source.
fn format(sig: &str) -> String {
    let lines: Vec<&str> =
        sig.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == lines.len() - 1 {
            out.push_str("sig_end_canary:\n");
        }
        out.push_str(&format!(".word 0x{line}\n"));
        if line.contains(FINAL_SIG_OFFSET_CANARY) {
            out.push_str("final_sig_offset:\n");
        }
        if line.contains(FINAL_TRAP_OFFSET_CANARY) {
            out.push_str("final_trap_sig_offset:\n");
        }
        if line.contains(TRAP_CANARY) {
            out.push_str("trap_sigptr:\n");
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: sigfmt <signature> <results>");
        exit(2);
    }
    let sig = fs::read_to_string(&args[1]).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", args[1]);
        exit(2);
    });
    if let Err(e) = fs::write(&args[2], format(&sig)) {
        eprintln!("cannot write {}: {e}", args[2]);
        exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn labels_follow_their_canaries() {
        let sig = "00000001\n4b8e2d17\n7a110ff5\nd3a91f6c\n00000002\n";
        assert_eq!(
            format(sig),
            ".word 0x00000001\n\
             .word 0x4b8e2d17\nfinal_sig_offset:\n\
             .word 0x7a110ff5\nfinal_trap_sig_offset:\n\
             .word 0xd3a91f6c\ntrap_sigptr:\n\
             sig_end_canary:\n.word 0x00000002\n"
        );
    }

    #[test]
    fn blank_lines_are_dropped() {
        assert_eq!(
            format("\n0000000a\n\n"),
            "sig_end_canary:\n.word 0x0000000a\n"
        );
    }
}
