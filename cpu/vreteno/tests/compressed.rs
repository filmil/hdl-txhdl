// SPDX-License-Identifier: Apache-2.0
//! Every compressed instruction, read two ways. The build assembles
//! all 49 152 halfwords whose low two bits are not both set and has the
//! GNU disassembler list them; this reads the listing, spells each
//! instruction it names with the thirty-two bit encoders, and asks
//! `isa::compressed` for the same answer. A halfword the disassembler
//! does not name must be one `compressed` refuses.
//!
//! The disassembler and the RV32C specification differ in two places,
//! and the specification is followed. It names `c.addi16sp` of zero,
//! which the specification reserves, and the shifts by 32 or more,
//! which RV32C leaves to custom extensions.
use vreteno32::isa::*;

/// A register operand, `x12`.
fn reg(s: &str) -> u32 {
    s.trim()
        .strip_prefix('x')
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("not a register: {s}"))
}

/// A number operand, decimal or `0x` hexadecimal.
fn num(s: &str) -> i64 {
    let s = s.trim();
    match s.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).unwrap(),
        None => s.parse().unwrap_or_else(|_| panic!("not a number: {s}")),
    }
}

/// `imm(xN)`, as the offset and the register.
fn mem(s: &str) -> (i32, u32) {
    let (imm, rest) = s.split_once('(').unwrap();
    (num(imm) as i32, reg(rest.trim_end_matches(')')))
}

/// A branch or jump's target, printed as an address, as an offset
/// from the instruction's own.
fn target(s: &str, at: u32) -> i32 {
    let addr =
        u32::from_str_radix(s.split_whitespace().next().unwrap(), 16).unwrap();
    addr.wrapping_sub(at) as i32
}

/// What the disassembler's line says the halfword is, in thirty-two
/// bits, or `None` for one it does not name or the specification
/// reserves.
fn spelt(mnemonic: &str, ops: &str, at: u32) -> Option<u32> {
    let o: Vec<&str> = ops.split(',').map(str::trim).collect();
    Some(match mnemonic {
        "c.nop" => addi(0, 0, 0),
        "c.addi" => addi(reg(o[0]), reg(o[0]), num(o[1]) as i32),
        "c.li" => addi(reg(o[0]), 0, num(o[1]) as i32),
        "c.lui" => lui(reg(o[0]), num(o[1]) as u32),
        "c.addi16sp" if num(o[1]) == 0 => return None,
        "c.addi16sp" => addi(2, 2, num(o[1]) as i32),
        "c.addi4spn" => addi(reg(o[0]), 2, num(o[2]) as i32),
        "c.slli" | "c.srli" | "c.srai" if num(o[1]) >= 32 => return None,
        "c.slli" => slli(reg(o[0]), reg(o[0]), num(o[1]) as u32),
        "c.srli" => srli(reg(o[0]), reg(o[0]), num(o[1]) as u32),
        "c.srai" => srai(reg(o[0]), reg(o[0]), num(o[1]) as u32),
        // A shift by zero, which the disassembler names by the width
        // of the machine it would matter on: a hint.
        "c.slli64" => slli(reg(o[0]), reg(o[0]), 0),
        "c.srli64" => srli(reg(o[0]), reg(o[0]), 0),
        "c.srai64" => srai(reg(o[0]), reg(o[0]), 0),
        "c.andi" => andi(reg(o[0]), reg(o[0]), num(o[1]) as i32),
        "c.sub" => sub(reg(o[0]), reg(o[0]), reg(o[1])),
        "c.xor" => xor(reg(o[0]), reg(o[0]), reg(o[1])),
        "c.or" => or(reg(o[0]), reg(o[0]), reg(o[1])),
        "c.and" => and(reg(o[0]), reg(o[0]), reg(o[1])),
        "c.mv" => add(reg(o[0]), 0, reg(o[1])),
        "c.add" => add(reg(o[0]), reg(o[0]), reg(o[1])),
        "c.j" => jal(0, target(o[0], at)),
        "c.jal" => jal(1, target(o[0], at)),
        "c.beqz" => beq(reg(o[0]), 0, target(o[1], at)),
        "c.bnez" => bne(reg(o[0]), 0, target(o[1], at)),
        "c.lw" | "c.lwsp" => {
            let (imm, base) = mem(o[1]);
            lw(reg(o[0]), base, imm)
        }
        "c.sw" | "c.swsp" => {
            let (imm, base) = mem(o[1]);
            sw(reg(o[0]), base, imm)
        }
        "c.jr" => jalr(0, reg(o[0]), 0),
        "c.jalr" => jalr(1, reg(o[0]), 0),
        "c.ebreak" => ebreak(),
        _ => return None,
    })
}

#[test]
fn every_halfword_agrees_with_the_gnu_disassembler() {
    let path = std::env::var("COMPRESSED_LISTING").unwrap();
    let listing = std::fs::read_to_string(&path).unwrap();
    let mut seen = 0;
    let mut named = 0;
    for line in listing.lines() {
        // `   addr:\thalf    \tmnemonic\toperands`
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 || !f[0].trim_end().ends_with(':') {
            continue;
        }
        let at =
            u32::from_str_radix(f[0].trim().trim_end_matches(':'), 16).unwrap();
        let half = u16::from_str_radix(f[1].trim(), 16).unwrap();
        // The operands end where a comment begins.
        let ops = f.get(3).map(|s| s.split('#').next().unwrap()).unwrap_or("");
        let want = spelt(f[2].trim(), ops, at);
        assert_eq!(
            compressed(half),
            want,
            "{half:#06x}: the disassembler says `{} {}`",
            f[2].trim(),
            ops.trim()
        );
        seen += 1;
        named += want.is_some() as usize;
    }
    assert_eq!(seen, 49152, "every compressed halfword, once");
    // The instructions RV32C has, hints included: the 30 360 the
    // disassembler names, less the one c.addi16sp and the 1 536 shifts
    // the specification reserves. A change to the listing, or to the
    // reading of it, shows here first.
    assert_eq!(named, 28823, "halfwords that are RV32C instructions");
}
