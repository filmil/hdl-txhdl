// SPDX-License-Identifier: Apache-2.0
//! An ELF for Vreteno, turned into the two images the core is loaded
//! from, and checked against what the core can actually run.
//!
//! The checks are the point. Every one of them is something the core
//! does silently and wrongly rather than refusing: a program past
//! 4096 bytes is truncated by the loader and the fetch wraps into it;
//! an entry point other than zero is never reached, since the core
//! starts at zero; a constant left in instruction memory is
//! unreadable, because a load from below the data base never leaves
//! the core. Each of those is caught here, where it can be said out
//! loud, rather than in a simulation that quietly does the wrong
//! thing.
//!
//! Usage: `elf2vreteno IMAGE.elf > image.rs`
use std::fmt::Write as _;

/// Instruction memory: 1024 words at zero, read by the fetch alone.
const IMEM_BASE: u32 = 0x0000;
const IMEM_BYTES: u32 = 4096;
/// Data memory: 1024 words at `0x1000`, a device on the bus.
const DMEM_BASE: u32 = 0x1000;
const DMEM_BYTES: u32 = 4096;

const SHF_ALLOC: u32 = 0x2;
const SHT_NOBITS: u32 = 8;

#[derive(Debug)]
struct Section {
    name: String,
    typ: u32,
    flags: u32,
    addr: u32,
    off: u32,
    size: u32,
}

fn u16le(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([d[at], d[at + 1]])
}

fn u32le(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn sections(d: &[u8]) -> Result<(u32, Vec<Section>), String> {
    if d.len() < 52 || &d[..4] != b"\x7fELF" {
        return Err("not an ELF file".into());
    }
    if d[4] != 1 {
        return Err("not a 32-bit ELF; Vreteno is a 32-bit machine".into());
    }
    if d[5] != 1 {
        return Err("not little-endian".into());
    }
    // e_machine 243 is RISC-V.
    let machine = u16le(d, 18);
    if machine != 243 {
        return Err(format!("e_machine is {machine}, not RISC-V (243)"));
    }
    let entry = u32le(d, 24);
    let shoff = u32le(d, 32) as usize;
    let shentsize = u16le(d, 46) as usize;
    let shnum = u16le(d, 48) as usize;
    let shstrndx = u16le(d, 50) as usize;
    let raw: Vec<[u32; 6]> = (0..shnum)
        .map(|i| {
            let o = shoff + i * shentsize;
            [
                u32le(d, o),
                u32le(d, o + 4),
                u32le(d, o + 8),
                u32le(d, o + 12),
                u32le(d, o + 16),
                u32le(d, o + 20),
            ]
        })
        .collect();
    let strbase = raw[shstrndx][4] as usize;
    let name_at = |off: u32| -> String {
        let start = strbase + off as usize;
        let end = d[start..].iter().position(|&b| b == 0).unwrap_or(0) + start;
        String::from_utf8_lossy(&d[start..end]).into_owned()
    };
    Ok((
        entry,
        raw.iter()
            .map(|r| Section {
                name: name_at(r[0]),
                typ: r[1],
                flags: r[2],
                addr: r[3],
                off: r[4],
                size: r[5],
            })
            .collect(),
    ))
}

fn run() -> Result<String, String> {
    let path = std::env::args().nth(1).ok_or("usage: elf2vreteno FILE")?;
    let d = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let (entry, secs) = sections(&d)?;
    if entry != IMEM_BASE {
        return Err(format!(
            "the entry point is {entry:#x}, and the core starts at \
             {IMEM_BASE:#x}; put the entry stub in `.text.init`"
        ));
    }
    let mut imem = vec![0u8; 0];
    let mut dmem = vec![0u8; 0];
    let put = |into: &mut Vec<u8>, at: usize, bytes: &[u8]| {
        if into.len() < at + bytes.len() {
            into.resize(at + bytes.len(), 0);
        }
        into[at..at + bytes.len()].copy_from_slice(bytes);
    };
    for s in &secs {
        if s.flags & SHF_ALLOC == 0 || s.size == 0 {
            continue;
        }
        let end = s.addr + s.size;
        // The instruction memory starts at zero, so every address is at
        // or above its base.
        let in_imem = end <= IMEM_BASE + IMEM_BYTES;
        let in_dmem = s.addr >= DMEM_BASE && end <= DMEM_BASE + DMEM_BYTES;
        if !in_imem && !in_dmem {
            return Err(format!(
                "section `{}` is at {:#x}..{:#x}, which is neither the \
                 instruction memory ({:#x}..{:#x}) nor the data memory \
                 ({:#x}..{:#x})",
                s.name,
                s.addr,
                end,
                IMEM_BASE,
                IMEM_BASE + IMEM_BYTES,
                DMEM_BASE,
                DMEM_BASE + DMEM_BYTES
            ));
        }
        // A section in the instruction memory need not hold code: the
        // boot memory is on the bus as well, read-only, so a constant
        // placed beside the code is a constant a load can reach. What
        // goes into which image is decided by address, not by flags.
        if s.typ == SHT_NOBITS {
            continue; // `.bss`: space, not bytes. The stub zeroes it.
        }
        let bytes = &d[s.off as usize..(s.off + s.size) as usize];
        if in_imem {
            put(&mut imem, (s.addr - IMEM_BASE) as usize, bytes);
        } else {
            put(&mut dmem, (s.addr - DMEM_BASE) as usize, bytes);
        }
    }
    if imem.is_empty() {
        return Err("the image has no instructions".into());
    }
    if !imem.len().is_multiple_of(4) {
        imem.resize(imem.len().div_ceil(4) * 4, 0);
    }
    if imem.len() > IMEM_BYTES as usize {
        return Err(format!(
            "the program is {} bytes and the instruction memory is {}; \
             the loader would truncate it and the fetch would wrap",
            imem.len(),
            IMEM_BYTES
        ));
    }
    if dmem.len() > DMEM_BYTES as usize {
        return Err(format!(
            "the initialised data is {} bytes and the data memory is {}",
            dmem.len(),
            DMEM_BYTES
        ));
    }
    let words: Vec<u32> = imem
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let mut out = String::new();
    writeln!(out, "// SPDX-License-Identifier: Apache-2.0").unwrap();
    writeln!(
        out,
        "//! An image for Vreteno, written by `//tools/elf2vreteno` \
         from a\n//! linked ELF. Do not edit: it is a build output, \
         and the tool\n//! checks it against what the core can run."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// The program: {} words, of the 1024 there is room for, \
         holding\n/// instructions of sixteen and thirty-two bits.",
        words.len()
    )
    .unwrap();
    writeln!(out, "pub const TEXT: &[u32] = &[").unwrap();
    for c in words.chunks(6) {
        let row: Vec<String> = c.iter().map(|w| format!("{w:#010x}")).collect();
        writeln!(out, "    {},", row.join(", ")).unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// What the data memory holds before the first cycle: {} \
         bytes\n/// at {:#x}, the constants the program reads.",
        dmem.len(),
        DMEM_BASE
    )
    .unwrap();
    writeln!(out, "pub const DATA: &[u8] = &[").unwrap();
    for c in dmem.chunks(12) {
        let row: Vec<String> = c.iter().map(|b| format!("{b:#04x}")).collect();
        writeln!(out, "    {},", row.join(", ")).unwrap();
    }
    writeln!(out, "];").unwrap();
    Ok(out)
}

fn main() {
    match run() {
        Ok(s) => print!("{s}"),
        Err(e) => {
            eprintln!("elf2vreteno: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sections;

    /// Anything that is not a 32-bit little-endian RISC-V ELF is
    /// refused by name rather than misread.
    #[test]
    fn only_a_riscv_elf_is_taken() {
        assert!(sections(b"not an elf at all").is_err(), "not an ELF");
        let mut d = vec![0u8; 64];
        d[..4].copy_from_slice(b"\x7fELF");
        d[4] = 2; // 64-bit
        d[5] = 1;
        let e = sections(&d).unwrap_err();
        assert!(e.contains("32-bit"), "said {e}");
    }
}
